/* ---------- 语音识别：whisper-server 常驻 + CLI 兜底 ---------- */
/* v0.4.0 效率重构：
   1. 常驻 whisper-server（模型只加载一次，免去每次识别 2-8s 的进程冷启动+模型加载）
   2. 量化模型 q5_1（487MB -> 180MB，推理更快，学校低配设备友好）
   3. 参数调优：greedy 解码（-bs 1 -bo 1）、中文 prompt、无时间戳
   4. server.json 记录 port/pid/model —— 旧实例孤儿 server 可被新实例复用，防重复驻留
   5. server 两连失败自动降级 CLI（保底可用性） */

use base64::Engine;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU16, Ordering};
use tauri::Manager;
use tokio::process::Command;

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

/* ---------- 模型档位 ---------- */

/// fast = q5_1 量化（默认，学校设备友好）；quality = fp16 原版
fn model_file(tier: &str) -> &'static str {
    match tier {
        "quality" => "ggml-small.bin",
        _ => "ggml-small-q5_1.bin",
    }
}

fn resolve_model(tier: &str) -> Option<PathBuf> {
    let name = model_file(tier);
    if let Some(p) = find_tool(&format!("tools/models/{name}")) {
        return Some(p);
    }
    // 档位文件缺失时回退另一档
    let alt = if name == "ggml-small-q5_1.bin" { "ggml-small.bin" } else { "ggml-small-q5_1.bin" };
    find_tool(&format!("tools/models/{alt}"))
}

/// 模型目录 + 纯 ASCII 相对文件名。
/// whisper 二进制按 UTF-8 解释 argv：绝对路径含中文时（GBK 字节非法 UTF-8）
/// whisper_model_load 直接 fail-fast（0xC0000409），因此模型必须以相对名传入。
fn model_dir_and_name(tier: &str) -> Option<(PathBuf, String)> {
    let name = model_file(tier);
    if let Some(p) = find_tool(&format!("tools/models/{name}")) {
        return Some((p.parent()?.to_path_buf(), name.to_string()));
    }
    // 档位文件缺失时回退另一档
    let alt = if name == "ggml-small-q5_1.bin" { "ggml-small.bin" } else { "ggml-small-q5_1.bin" };
    let p = find_tool(&format!("tools/models/{alt}"))?;
    Some((p.parent()?.to_path_buf(), alt.to_string()))
}

fn whisper_dir() -> Option<PathBuf> {
    find_tool("tools/whisper/Release/whisper-cli.exe").map(|p| p.parent().unwrap().to_path_buf())
}

/* ---------- 通用参数 ---------- */

/// 中文识别的公共调优参数（CLI 与 server 共用的语义）
/// -l zh 固定语言（跳过自动检测）；--prompt 中文引导；-nt 无时间戳
fn common_args(cfg: &crate::config::Config, out: &mut Vec<String>) {
    let lang = if cfg.voice_lang.is_empty() { "zh" } else { cfg.voice_lang.as_str() };
    if lang != "auto" {
        out.push("-l".into()); out.push(lang.into());
    } // auto：省略 -l 让 whisper 自动检测（略慢）
    // greedy 解码：beam=1 best=1，对 3-15s 短指令基本无损，速度显著提升
    out.push("-bs".into()); out.push("1".into());
    out.push("-bo".into()); out.push("1".into());
    // 注意：不再经 argv 传中文 prompt——argv 经 CRT 转 GBK 后被 whisper 按 UTF-8
    // 解释成乱码（还可能触发 fail-fast）。server 模式的 prompt 改走 HTTP 表单（UTF-8 安全）。
    // 线程数：手动配置优先；否则按逻辑核一半自适应（低配设备留核保系统流畅，clamp 2-4）
    let t = if cfg.voice_threads > 0 {
        cfg.voice_threads
    } else {
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        (n / 2).clamp(2, 4) as i32
    };
    if t > 0 {
        out.push("-t".into()); out.push(t.to_string());
    }
}

/* ---------- server 管理 ---------- */

static SERVER_PORT: AtomicU16 = AtomicU16::new(0);
static SERVER_LAST_USED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn touch_server() {
    SERVER_LAST_USED.store(now_secs(), Ordering::Relaxed);
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ServerInfo {
    port: u16,
    pid: u32,
    model: String,
    #[serde(default)]
    lang: String,
}

fn server_info_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("server.json"))
}

fn read_server_info(app: &tauri::AppHandle) -> Option<ServerInfo> {
    let p = server_info_path(app)?;
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

fn write_server_info(app: &tauri::AppHandle, info: &ServerInfo) {
    if let Some(p) = server_info_path(app) {
        if let Ok(j) = serde_json::to_string(info) {
            let _ = std::fs::write(p, j);
        }
    }
}

fn kill_pid(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .creation_flags(0x08000000)
            .output();
    }
}

/// 探活：有 HTTP 响应（任意状态码）即认为 server 活着
fn probe(port: u16) -> bool {
    let addr = format!("127.0.0.1:{port}");
    std::net::TcpStream::connect(&addr)
        .map(|s| {
            // TCP 通了还不够稳，发一个最小 HTTP 请求确认是我们的 server
            use std::io::{Read, Write};
            let mut s = s;
            let _ = s.write_all(format!("GET / HTTP/1.0\r\nHost: {addr}\r\n\r\n").as_bytes());
            let mut buf = [0u8; 16];
            matches!(s.read(&mut buf), Ok(n) if n > 0)
        })
        .unwrap_or(false)
}

/// 确保有可用的 whisper-server，返回端口。复用旧实例（含孤儿），失败则启动新的。
async fn ensure_server(cfg: &crate::config::Config) -> Result<u16, String> {
    let model_name = model_file(cfg.voice_model.as_str()).to_string();
    let lang = if cfg.voice_lang.is_empty() { "zh".to_string() } else { cfg.voice_lang.clone() };

    // 1. 本进程已知端口
    let known = SERVER_PORT.load(Ordering::Relaxed);
    if known != 0 && probe(known) {
        touch_server();
        return Ok(known);
    }

    // 2. 跨实例复用（server.json：可能是上次实例留下的健康孤儿）
    if let Some(app) = crate::APP_HANDLE.get() {
        if let Some(info) = read_server_info(app) {
            if info.model == model_name && info.lang == lang && probe(info.port) {
                SERVER_PORT.store(info.port, Ordering::Relaxed);
                touch_server();
                return Ok(info.port);
            }
            // 存在但不健康或档位不符 → 清理
            kill_pid(info.pid);
        }
    }

    // 3. 启动新 server（模型以 cwd=models + 相对名传入，规避非 ASCII 路径崩溃）
    let dir = whisper_dir().ok_or("未找到 tools/whisper/Release/")?;
    let (models_cwd, model_name) = model_dir_and_name(&cfg.voice_model)
        .ok_or("未找到语音模型文件（tools/models/）")?;

    // 选空闲端口
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        l.local_addr().map_err(|e| e.to_string())?.port()
    };

    let mut args: Vec<String> = vec![
        "-m".into(), model_name.clone(),
        "--host".into(), "127.0.0.1".into(),
        "--port".into(), port.to_string(),
        "-nt".into(),
    ];
    common_args(cfg, &mut args);

    // stderr 落盘：server 崩溃不再静默，事后可查 whisper-server.log
    let log_file = crate::APP_HANDLE.get().and_then(|app| {
        let d = app.path().app_config_dir().ok()?;
        let _ = std::fs::create_dir_all(&d);
        std::fs::File::create(d.join("whisper-server.log")).ok()
    });

    let mut cmd = Command::new(dir.join("whisper-server.exe"));
    cmd.args(&args)
        .current_dir(&models_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    match log_file {
        Some(f) => { cmd.stderr(Stdio::from(f)); }
        None => { cmd.stderr(Stdio::null()); }
    }
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().map_err(|e| format!("whisper-server 启动失败: {e}"))?;
    let pid = child.id().unwrap_or(0);

    // 早退检测：进程一退出立即读日志报错，不再傻等 30s
    let waiter = tokio::spawn(async move { child.wait().await });

    // 等待就绪（模型加载：q5_1 SSD 约 1-3s，HDD/低配机放宽到 30s）
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if probe(port) {
            SERVER_PORT.store(port, Ordering::Relaxed);
            touch_server();
            if let Some(app) = crate::APP_HANDLE.get() {
                write_server_info(app, &ServerInfo { port, pid, model: model_name, lang });
            }
            return Ok(port);
        }
        if waiter.is_finished() {
            let tail = crate::APP_HANDLE
                .get()
                .and_then(|app| {
                    let p = app.path().app_config_dir().ok()?.join("whisper-server.log");
                    std::fs::read_to_string(p).ok()
                })
                .map(|s| {
                    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
                    let start = lines.len().saturating_sub(4);
                    lines[start..].join(" | ")
                })
                .unwrap_or_default();
            kill_pid(pid);
            return Err(format!("whisper-server 启动即退出: {tail}"));
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    kill_pid(pid);
    Err("whisper-server 启动超时（详见配置目录 whisper-server.log）".into())
}

fn stop_server() {
    let p = SERVER_PORT.swap(0, Ordering::Relaxed);
    let _ = p;
    if let Some(app) = crate::APP_HANDLE.get() {
        if let Some(info) = read_server_info(app) {
            kill_pid(info.pid);
            if let Some(path) = server_info_path(app) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// 预热：应用启动后后台静默拉起常驻 server（用户首次说话即热态，无冷启动等待）
pub async fn warmup() {
    let cfg = match crate::APP_HANDLE.get() {
        Some(app) => crate::config::load(app),
        None => return,
    };
    let _ = ensure_server(&cfg).await;
}

/// 空闲回收：每 5 分钟检查，30 分钟无识别请求则关闭常驻 server
/// （低配教室机释放约 500MB 内存；下次语音时 ensure_server 会重新拉起）
pub fn start_idle_reaper() {
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(300));
            let last = SERVER_LAST_USED.load(Ordering::Relaxed);
            if last == 0 {
                continue; // 从未启动过（或已被回收）
            }
            if now_secs().saturating_sub(last) > 1800 {
                stop_server();
                SERVER_LAST_USED.store(0, Ordering::Relaxed);
                eprintln!("vcc: whisper server idle 30min, reclaimed");
            }
        }
    });
}

/* ---------- HTTP 推理 ---------- */

async fn infer_via_server(cfg: &crate::config::Config, port: u16, wav_path: &Path, timeout_secs: u64) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{port}/inference");
    let lang = if cfg.voice_lang.is_empty() { "zh".to_string() } else { cfg.voice_lang.clone() };
    let wav_bytes = std::fs::read(wav_path).map_err(|e| format!("读临时文件失败: {e}"))?;
    let boundary = "----vccform7d3a";
    let part = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
    );
    // prompt 走 HTTP 表单（UTF-8 安全）；argv 传中文会被 CRT 转 GBK 再按 UTF-8 解释成乱码
    let prompt_part = if lang == "zh" {
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\n以下是普通话的句子。\r\n")
    } else {
        String::new()
    };
    let body = part.into_bytes()
        .into_iter()
        .chain(wav_bytes)
        .chain(prompt_part.into_bytes())
        .chain(format!("\r\n--{boundary}--\r\n").into_bytes())
        .collect::<Vec<u8>>();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(&url)
        .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
        .body(body)
        .send()
        .await
        .map_err(|e| format!("server 请求失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("server HTTP {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    // response_format 默认 json：{"text": "..."}
    let parsed: Option<String> = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from));
    Ok(parsed.unwrap_or(text).trim().to_string())
}

/* ---------- CLI 兜底 ---------- */

async fn infer_via_cli(cfg: &crate::config::Config, wav_path: &Path) -> Result<String, String> {
    let dir = whisper_dir().ok_or("未找到 tools/whisper/Release/")?;
    let (models_cwd, model_name) = model_dir_and_name(&cfg.voice_model)
        .ok_or("未找到语音模型文件（tools/models/）")?;

    // wav 路径同样须 ASCII：临时目录含非 ASCII（如中文用户名）时拷入模型目录用相对名
    let mut wav_arg = wav_path.to_string_lossy().into_owned();
    let mut copied: Option<PathBuf> = None;
    if wav_arg.chars().any(|c| (c as u32) > 127) {
        let alt = models_cwd.join("vcc_rec_tmp.wav");
        if std::fs::copy(wav_path, &alt).is_ok() {
            wav_arg = "vcc_rec_tmp.wav".into();
            copied = Some(alt);
        }
    }

    let mut args: Vec<String> = vec![
        "-m".into(), model_name,
        "-f".into(), wav_arg,
    ];
    common_args(cfg, &mut args);
    args.push("-np".into());

    let mut cmd = Command::new(dir.join("whisper-cli.exe"));
    cmd.args(&args)
        .current_dir(&models_cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let out = match cmd.output().await {
        Ok(o) => o,
        Err(e) => {
            if let Some(c) = &copied { let _ = std::fs::remove_file(c); }
            return Err(format!("whisper 启动失败: {e}"));
        }
    };
    if let Some(c) = &copied { let _ = std::fs::remove_file(c); }
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
        let start = tail.len().saturating_sub(5);
        let joined = tail[start..].join(" | ");
        let code = out.status.code().unwrap_or(0) as u32;
        return Err(format!("whisper 运行失败(0x{code:08X}): {joined}"));
    }
    let text = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(text.trim().to_string())
}

/* ---------- 对外入口 ---------- */

/// base64(WAV 16k mono) -> 文本。server 常驻优先，两连失败降级 CLI。
pub async fn transcribe(wav_base64: &str) -> Result<String, String> {
    use base64::engine::general_purpose::STANDARD;

    touch_server();
    let bytes = STANDARD
        .decode(wav_base64.trim())
        .map_err(|e| format!("WAV 解码失败: {e}"))?;
    if bytes.len() < 100 {
        return Err("录音数据为空".into());
    }

    let tmp = std::env::temp_dir().join(format!(
        "vcc_rec_{}.wav",
        std::process::id() as u64 + chrono_millis()
    ));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("写入临时文件失败: {e}"))?;

    let cfg = {
        match crate::APP_HANDLE.get() {
            Some(app) => crate::config::load(app),
            None => crate::config::Config::default(),
        }
    };

    let result = transcribe_inner(&cfg, &tmp).await;
    let _ = std::fs::remove_file(&tmp);
    result
}

async fn transcribe_inner(cfg: &crate::config::Config, tmp: &Path) -> Result<String, String> {
    // 1. 常驻 server（首次调用会启动并等模型加载，之后复用）
    for attempt in 0..2 {
        match ensure_server(cfg).await {
            Ok(port) => match infer_via_server(cfg, port, tmp, 60).await {
                Ok(t) => return finalize_text(t),
                Err(e) => {
                    if attempt == 0 {
                        // server 可能死掉/卡死：清理后重启重试一次
                        stop_server();
                        continue;
                    }
                    eprintln!("server 推理失败（已重试）: {e}");
                }
            },
            Err(e) => {
                eprintln!("server 启动失败（attempt {attempt}）: {e}");
                break;
            }
        }
    }
    // 2. CLI 兜底
    let t = infer_via_cli(cfg, tmp).await?;
    finalize_text(t)
}

fn finalize_text(t: String) -> Result<String, String> {
    let t = t.trim().to_string();
    Ok(t)
}

fn chrono_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_millis() as u64)
        .unwrap_or(0)
}

/// 预热检查 + 基准（供诊断：invoke('probe_env')）
#[tauri::command]
pub fn probe_env() -> Result<String, String> {
    let cli = whisper_dir().ok_or("tools/whisper/Release/ 未找到")?;
    let fast = resolve_model("fast").map(|p| p.display().to_string()).unwrap_or_default();
    let quality = resolve_model("quality").map(|p| p.display().to_string()).unwrap_or_default();
    Ok(format!(
        "whisper: {}\nfast(q5_1): {}\nquality(fp16): {}\nserver: 端口 {}",
        cli.display(),
        fast,
        quality,
        SERVER_PORT.load(Ordering::Relaxed)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：模型以 cwd=models + 相对名传入（绝对路径含中文会让 whisper fail-fast 崩溃）
    #[test]
    fn cli_transcribe_nonascii_repo() {
        let cfg = crate::config::Config::default();
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().to_path_buf();
        let src = repo.join("tools/tests/syn-3s.wav");
        if !src.exists() {
            return; // 无测试音频时跳过
        }
        // 拷到 %TEMP%（ASCII 路径）模拟真实录音临时文件
        let tmp = std::env::temp_dir().join("vcc_test_cli_reg.wav");
        std::fs::copy(&src, &tmp).unwrap();
        let t = tauri::async_runtime::block_on(infer_via_cli(&cfg, &tmp));
        let _ = std::fs::remove_file(&tmp);
        let t = t.expect("whisper CLI 推理应成功");
        assert!(!t.trim().is_empty(), "whisper CLI 应返回非空文本");
    }
}
