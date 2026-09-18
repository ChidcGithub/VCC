/* ---------- 语音识别：whisper-server 常驻 + CLI 兜底 ---------- */

use base64::Engine;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};

const MAX_AUDIO_BYTES: usize = 16_000 * 2 * 60;
const MAX_WAV_BYTES: usize = MAX_AUDIO_BYTES + 65_536;
const MAX_BASE64_BYTES: usize = ((MAX_WAV_BYTES + 2) / 3) * 4;
const CLI_TIMEOUT: Duration = Duration::from_secs(90);

/// 从 exe 目录逐级向上查找项目内的工具文件（dev 与打包布局都兼容）
pub fn find_tool(rel: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent().map(|p| p.to_path_buf())?;
    for _ in 0..7 {
        let candidate = dir.join(rel);
        if candidate.exists() {
            return Some(candidate);
        }
        dir = dir.parent()?.to_path_buf();
    }
    None
}

/* ---------- 模型档位与公共参数 ---------- */

fn model_file(tier: &str) -> &'static str {
    match tier {
        "quality" => "ggml-small.bin",
        _ => "ggml-small-q5_1.bin",
    }
}

fn resolve_model(tier: &str) -> Option<PathBuf> {
    let (dir, name) = model_dir_and_name(tier)?;
    Some(dir.join(name))
}

/// 模型以 cwd + ASCII 相对文件名传入，规避 Windows 非 ASCII argv 问题。
fn model_dir_and_name(tier: &str) -> Option<(PathBuf, String)> {
    let name = model_file(tier);
    if let Some(p) = find_tool(&format!("tools/models/{name}")) {
        return Some((p.parent()?.to_path_buf(), name.to_string()));
    }
    let alt = if name == "ggml-small-q5_1.bin" {
        "ggml-small.bin"
    } else {
        "ggml-small-q5_1.bin"
    };
    let p = find_tool(&format!("tools/models/{alt}"))?;
    Some((p.parent()?.to_path_buf(), alt.to_string()))
}

fn whisper_dir() -> Option<PathBuf> {
    find_tool("tools/whisper/Release/whisper-cli.exe")
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

fn language(cfg: &crate::config::Config) -> &str {
    if cfg.voice_lang.trim().is_empty() {
        "zh"
    } else {
        cfg.voice_lang.trim()
    }
}

fn threads(cfg: &crate::config::Config) -> i32 {
    if cfg.voice_threads > 0 {
        cfg.voice_threads
    } else {
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        (n / 2).clamp(2, 4) as i32
    }
}

fn common_args(lang: &str, threads: i32, out: &mut Vec<String>) {
    // auto 必须显式传入，whisper 的默认语言不一定是自动检测。
    out.extend([
        "-l".into(), lang.into(),
        "-bs".into(), "1".into(),
        "-bo".into(), "1".into(),
        "-t".into(), threads.to_string(),
    ]);
}

/* ---------- server 管理 ---------- */

static SERVER_PORT: AtomicU16 = AtomicU16::new(0);

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
struct ServerInfo {
    port: u16,
    pid: u32,
    model: String,
    #[serde(default)]
    lang: String,
    // 旧记录没有线程数，不能作为匹配的缓存使用。
    #[serde(default)]
    threads: i32,
}

fn cache_matches(info: &ServerInfo, actual_model: &str, lang: &str, threads: i32) -> bool {
    info.port != 0 && info.model == actual_model && info.lang == lang && info.threads == threads
}

struct ServerState {
    info: Option<ServerInfo>,
    child: Option<Child>,
    last_used: Option<Instant>,
}

// 锁不仅覆盖检查/启动，还覆盖一次 HTTP 推理及失败失效操作。
static ENSURING: tokio::sync::Mutex<ServerState> = tokio::sync::Mutex::const_new(ServerState {
    info: None,
    child: None,
    last_used: None,
});

struct ServerLease {
    state: tokio::sync::MutexGuard<'static, ServerState>,
}

impl Drop for ServerLease {
    fn drop(&mut self) {
        // 成功、错误、select 取消都从实际释放租约的时刻开始计算空闲时间。
        self.state.last_used = self.state.info.as_ref().map(|_| Instant::now());
    }
}

fn server_info_path() -> PathBuf {
    crate::config::data_dir().join("server.json")
}

fn read_server_info() -> Option<ServerInfo> {
    let mut text = String::new();
    std::fs::File::open(server_info_path()).ok()?.take(8193).read_to_string(&mut text).ok()?;
    if text.len() > 8192 { return None; }
    serde_json::from_str(&text).ok()
}

fn write_server_info(info: &ServerInfo) {
    if let Ok(json) = serde_json::to_string(info) {
        let _ = std::fs::write(server_info_path(), json);
    }
}

/// 只终止本进程持有句柄的子进程；绝不根据 server.json 中的 PID 杀进程。
fn stop_server(state: &mut ServerState) {
    let previous = state.info.take();
    SERVER_PORT.store(0, Ordering::Relaxed);
    state.last_used = None;
    if let Some(mut child) = state.child.take() {
        let _ = child.start_kill();
        // kill_on_drop 兜底，Tokio 负责子进程回收。
        drop(child);
        if previous.is_some() && read_server_info() == previous {
            let _ = std::fs::remove_file(server_info_path());
        }
    }
}

async fn probe(port: u16) -> bool {
    // 任意 HTTP 响应不代表 whisper；仅接受健康接口的 JSON 成功响应。
    let client = match reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let response = match client.get(format!("http://127.0.0.1:{port}/health")).send().await {
        Ok(response) if response.status().is_success() => response,
        _ => return false,
    };
    match response_text(response, 4096).await {
        Ok(text) => serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .is_some_and(|value| value.get("status").and_then(|s| s.as_str()) == Some("ok")),
        Err(_) => false,
    }
}

async fn ensure_server(cfg: &crate::config::Config, reuse_saved: bool) -> Result<ServerLease, String> {
    let mut lease = ServerLease { state: ENSURING.lock().await };
    // 必须先解析回退后的实际文件名，不能使用用户请求的档位名比较缓存。
    let (models_cwd, model_name) = model_dir_and_name(&cfg.voice_model)
        .ok_or("未找到语音模型文件（tools/models/）")?;
    let lang = language(cfg);
    let thread_count = threads(cfg);

    if let Some(info) = lease.state.info.clone() {
        let owned_alive = match lease.state.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => true,
        };
        if owned_alive && cache_matches(&info, &model_name, lang, thread_count) && probe(info.port).await {
            return Ok(lease);
        }
        stop_server(&mut lease.state);
    }

    if reuse_saved {
        if let Some(info) = read_server_info() {
            if cache_matches(&info, &model_name, lang, thread_count) && probe(info.port).await {
                SERVER_PORT.store(info.port, Ordering::Relaxed);
                lease.state.info = Some(info);
                return Ok(lease);
            }
            // 非本进程拥有的旧实例不清理；另选空闲端口，避免 PID 复用误杀。
        }
    }

    let dir = whisper_dir().ok_or("未找到 tools/whisper/Release/")?;
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.local_addr().map_err(|e| e.to_string())?.port()
    };
    let mut args = vec![
        "-m".into(), model_name.clone(),
        "--host".into(), "127.0.0.1".into(),
        "--port".into(), port.to_string(), "-nt".into(),
    ];
    common_args(lang, thread_count, &mut args);
    let log_file = std::fs::File::create(crate::config::data_dir().join("whisper-server.log")).ok();
    let mut cmd = Command::new(dir.join("whisper-server.exe"));
    cmd.args(args)
        .current_dir(models_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log_file.map(Stdio::from).unwrap_or_else(Stdio::null))
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    // 就绪前只由此 future 持有子进程，取消或超时会自动终止，不留下脱管 waiter。
    let mut child = cmd.spawn().map_err(|e| format!("whisper-server 启动失败: {e}"))?;
    let ready = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(status) = child.try_wait().map_err(|e| format!("读取 server 状态失败: {e}"))? {
                return Err(format!("whisper-server 启动即退出（{status}，详见 whisper-server.log）"));
            }
            if probe(port).await {
                // 探活期间子进程可能退出，不能将别的端口占用者登记为自己启动的实例。
                if matches!(child.try_wait(), Ok(None)) { return Ok(()); }
                return Err("whisper-server 就绪检查时已退出".into());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }).await;
    ready.map_err(|_| "whisper-server 启动超时（详见 whisper-server.log）".to_string())??;

    let info = ServerInfo {
        port, pid: child.id().unwrap_or(0), model: model_name,
        lang: lang.into(), threads: thread_count,
    };
    write_server_info(&info);
    SERVER_PORT.store(port, Ordering::Relaxed);
    lease.state.info = Some(info);
    lease.state.child = Some(child);
    Ok(lease)
}

pub async fn warmup() {
    let cfg = crate::config::load();
    let _ = ensure_server(&cfg, true).await;
}

/// 回收检查与启动/推理共享锁，忙碌时跳过；不终止其他应用实例的 server。
pub fn start_idle_reaper() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(300));
        if let Ok(mut state) = ENSURING.try_lock() {
            if state.last_used.is_some_and(|last| last.elapsed() > Duration::from_secs(1800)) {
                stop_server(&mut state);
                eprintln!("vcc: 语音 server 空闲缓存已回收");
            }
        }
    });
}

/* ---------- WAV 校验与临时文件 ---------- */

/// 严格检查 RIFF 块边界及 PCM 格式，允许合法的附加块和奇数长度块 padding。
fn validate_wav(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_WAV_BYTES { return Err("录音数据过大，最多支持 60 秒".into()); }
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("录音不是有效的 RIFF/WAV 文件".into());
    }
    let u32_at = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let u16_at = |offset: usize| u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
    if u32_at(4) as u64 + 8 != bytes.len() as u64 {
        return Err("WAV 头声明的文件长度不一致".into());
    }
    let mut offset = 12usize;
    let mut have_fmt = false;
    let mut have_data = false;
    while offset < bytes.len() {
        if bytes.len() - offset < 8 { return Err("WAV 块头不完整".into()); }
        let len = u32_at(offset + 4) as usize;
        let start = offset + 8;
        let end = start.checked_add(len).filter(|end| *end <= bytes.len())
            .ok_or("WAV 数据块越界或被截断")?;
        match &bytes[offset..offset + 4] {
            b"fmt " => {
                if have_fmt || len < 16 { return Err("WAV 格式块无效或重复".into()); }
                if u16_at(start) != 1 || u16_at(start + 2) != 1 || u32_at(start + 4) != 16_000
                    || u32_at(start + 8) != 32_000 || u16_at(start + 12) != 2 || u16_at(start + 14) != 16
                {
                    return Err("录音须为 16kHz、单声道、16bit PCM WAV".into());
                }
                have_fmt = true;
            }
            b"data" => {
                if !have_fmt || have_data || len == 0 || len % 2 != 0 {
                    return Err("WAV 音频块为空、重复、顺序或长度无效".into());
                }
                if len > MAX_AUDIO_BYTES { return Err("录音超过 60 秒上限".into()); }
                have_data = true;
            }
            _ => {}
        }
        offset = end.checked_add(len % 2).filter(|end| *end <= bytes.len())
            .ok_or("WAV 数据块缺少对齐字节")?;
    }
    if !have_fmt || !have_data { return Err("WAV 缺少格式块或音频数据块".into()); }
    Ok(())
}

fn read_wav(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("读取录音失败: {e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_WAV_BYTES as u64 + 1).read_to_end(&mut bytes)
        .map_err(|e| format!("读取录音失败: {e}"))?;
    validate_wav(&bytes)?;
    Ok(bytes)
}

static WAV_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempWav {
    path: PathBuf,
}

impl TempWav {
    fn create_in(dir: &Path, bytes: &[u8]) -> Result<Self, String> {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default().as_nanos();
        for _ in 0..128 {
            let path = dir.join(format!("vcc_rec_{}_{}_{}.wav", std::process::id(), stamp,
                WAV_SEQ.fetch_add(1, Ordering::Relaxed)));
            let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("创建临时音频失败: {e}")),
            };
            let temp = Self { path };
            let result = file.write_all(bytes);
            // Windows 下必须先关闭句柄，再让错误路径上的 RAII 删除文件。
            drop(file);
            result.map_err(|e| format!("写入临时音频失败: {e}"))?;
            return Ok(temp);
        }
        Err("无法分配唯一的临时音频文件".into())
    }
}

impl Drop for TempWav {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() == std::io::ErrorKind::NotFound { return; }
            // CLI 取消时 kill_on_drop 已发出终止，但 Windows 文件句柄可能尚未释放。
            // 不依赖被取消的 Tokio future/运行时，有限重试清理唯一的自有文件。
            let path = self.path.clone();
            let _ = std::thread::Builder::new().name("vcc-audio-cleanup".into()).spawn(move || {
                for _ in 0..100 {
                    std::thread::sleep(Duration::from_millis(50));
                    match std::fs::remove_file(&path) {
                        Ok(()) => return,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                        Err(_) => {}
                    }
                }
                eprintln!("vcc: 临时音频清理失败: {}", path.display());
            });
        }
    }
}

/* ---------- HTTP 推理 ---------- */

/// 纯函数：选取不与载荷碰撞的 boundary，每一部分（包括 WAV）都以 CRLF 结束。
fn multipart_body(wav: &[u8], lang: &str) -> (String, Vec<u8>) {
    let mut index = 0u64;
    let boundary = loop {
        let candidate = format!("vcc-form-{index:x}");
        if !wav.windows(candidate.len()).any(|part| part == candidate.as_bytes())
            && !lang.contains(&candidate)
        {
            break candidate;
        }
        index += 1;
    };
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\n").into_bytes();
    body.extend_from_slice(wav);
    body.extend_from_slice(b"\r\n");
    for (name, value) in [("language", lang), ("response_format", "json")] {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    if lang == "zh" {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\n以下是普通话的句子。\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

fn parse_server_text(body: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| "语音 server 返回的不是合法 JSON".to_string())?;
    let object = value.as_object().ok_or("语音 server 返回的 JSON 必须是对象")?;
    if object.get("error").is_some_and(|error| !error.is_null()) {
        return Err("语音 server 返回了错误，未获得有效转写".into());
    }
    object.get("text").and_then(|text| text.as_str()).map(|text| text.trim().to_string())
        .ok_or_else(|| "语音 server 返回的 JSON 缺少字符串 text 字段".into())
}

async fn response_text(mut response: reqwest::Response, limit: usize) -> Result<String, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| format!("读取 server 响应失败: {e}"))? {
        if chunk.len() > limit - bytes.len() { return Err("语音 server 响应过大".into()); }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "语音 server 响应不是 UTF-8".into())
}

async fn infer_via_server(cfg: &crate::config::Config, port: u16, wav_path: &Path, timeout_secs: u64) -> Result<String, String> {
    let wav = read_wav(wav_path)?;
    let (boundary, body) = multipart_body(&wav, language(cfg));
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(timeout_secs))
        .build().map_err(|e| e.to_string())?;
    let response = client.post(format!("http://127.0.0.1:{port}/inference"))
        .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
        .body(body).send().await.map_err(|e| format!("server 请求失败: {e}"))?;
    if !response.status().is_success() { return Err(format!("server HTTP {}", response.status())); }
    parse_server_text(&response_text(response, 65_536).await?)
}

/* ---------- CLI 兜底 ---------- */

pub async fn infer_via_cli(cfg: &crate::config::Config, wav_path: &Path) -> Result<String, String> {
    let bytes = read_wav(wav_path)?;
    let dir = whisper_dir().ok_or("未找到 tools/whisper/Release/")?;
    let (models_cwd, model_name) = model_dir_and_name(&cfg.voice_model)
        .ok_or("未找到语音模型文件（tools/models/）")?;
    // 相对路径也复制，避免改变 cwd 后引用错文件；复制失败直接报告，不回退固定文件名。
    let copied = if !wav_path.is_absolute() || !wav_path.to_str().is_some_and(str::is_ascii) {
        Some(TempWav::create_in(&models_cwd, &bytes)?)
    } else {
        None
    };
    let wav_arg = match &copied {
        Some(temp) => temp.path.file_name().unwrap().to_string_lossy().into_owned(),
        None => wav_path.to_string_lossy().into_owned(),
    };
    let mut args = vec!["-m".into(), model_name, "-f".into(), wav_arg];
    common_args(language(cfg), threads(cfg), &mut args);
    args.push("-np".into());
    let mut cmd = Command::new(dir.join("whisper-cli.exe"));
    cmd.args(args).current_dir(models_cwd)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let out = tokio::time::timeout(CLI_TIMEOUT, cmd.output()).await
        .map_err(|_| "语音识别超过 90 秒，已终止 CLI".to_string())?
        .map_err(|e| format!("whisper 启动失败: {e}"))?;
    if !out.status.success() {
        let error = String::from_utf8_lossy(&out.stderr);
        let lines: Vec<&str> = error.lines().filter(|line| !line.trim().is_empty()).collect();
        let detail = lines[lines.len().saturating_sub(5)..].join(" | ");
        let code = out.status.code().unwrap_or(0) as u32;
        return Err(format!("whisper 运行失败(0x{code:08X}): {detail}"));
    }
    let text = String::from_utf8_lossy(&out.stdout).lines().map(str::trim)
        .filter(|line| !line.is_empty()).collect::<Vec<_>>().join(" ");
    Ok(text.trim().to_string())
}

/* ---------- 对外入口 ---------- */

pub async fn transcribe(wav_base64: &str) -> Result<String, String> {
    use base64::engine::general_purpose::STANDARD;
    // 解码前限制分配量（含外围空白），解码后再校验头和实际音频时长。
    if wav_base64.len() > MAX_BASE64_BYTES { return Err("录音数据过大，最多支持 60 秒".into()); }
    let bytes = STANDARD.decode(wav_base64.trim()).map_err(|e| format!("WAV 解码失败: {e}"))?;
    validate_wav(&bytes)?;
    let temp = TempWav::create_in(&std::env::temp_dir(), &bytes)?;
    let cfg = crate::config::load();
    // temp 由 future 持有：包括 select 取消，任意退出路径都会执行 Drop。
    transcribe_inner(&cfg, &temp.path).await
}

async fn transcribe_inner(cfg: &crate::config::Config, tmp: &Path) -> Result<String, String> {
    for attempt in 0..2 {
        match ensure_server(cfg, attempt == 0).await {
            Ok(mut lease) => {
                let port = lease.state.info.as_ref().unwrap().port;
                match infer_via_server(cfg, port, tmp, 60).await {
                    Ok(text) => return finalize_text(text),
                    Err(error) => {
                        // 失效的是当前租约的实例，不会清掉另一请求刚启动的 server。
                        stop_server(&mut lease.state);
                        eprintln!("server 推理失败（第 {} 次）: {error}", attempt + 1);
                    }
                }
            }
            Err(error) => {
                eprintln!("server 启动失败: {error}");
                break;
            }
        }
    }
    finalize_text(infer_via_cli(cfg, tmp).await?)
}

fn finalize_text(text: String) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() { return Err("未识别到有效语音，请靠近麦克风后重试".into()); }
    Ok(text)
}

/// 仅检查工具路径与缓存端口，不启动录音或推理。
pub fn probe_env() -> Result<String, String> {
    let cli = whisper_dir().ok_or("tools/whisper/Release/ 未找到")?;
    let fast = resolve_model("fast").map(|p| p.display().to_string()).unwrap_or_default();
    let quality = resolve_model("quality").map(|p| p.display().to_string()).unwrap_or_default();
    Ok(format!("whisper: {}\nfast(q5_1): {}\nquality(fp16): {}\nserver: 端口 {}",
        cli.display(), fast, quality, SERVER_PORT.load(Ordering::Relaxed)))
}
