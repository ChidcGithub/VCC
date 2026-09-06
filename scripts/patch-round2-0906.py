# -*- coding: utf-8 -*-
"""第二轮补丁：复查+深挖发现的高优先级问题"""
import io, sys, os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PATCHES = [
    # C1: config load 并入 .bad 防护（load_safe 之前没有调用点，等于没生效）
    ("src-tauri/src/config.rs",
"""pub fn load(app: &AppHandle) -> Config {
    config_path(app).ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// load 的损坏防护版：解析失败时把坏文件改名 .bad 保留现场再回落默认
pub fn load_safe(app: &AppHandle) -> Config {
    let parsed = config_path(app).ok().and_then(|p| {
        fs::read_to_string(&p)
            .ok()
            .map(|s| (p, serde_json::from_str::<Config>(&s).ok()))
    });
    match parsed {
        Some((_, Some(cfg))) => cfg,
        Some((p, None)) => {
            let _ = fs::rename(&p, p.with_extension("json.bad"));
            Config::default()
        }
        None => Config::default(),
    }
}""",
"""/// 读取配置；解析失败时把坏文件改名 .bad 保留现场再回落默认
/// （否则下次 save 用默认配置覆盖，API Key 被静默抹掉）
pub fn load(app: &AppHandle) -> Config {
    let parsed = config_path(app).ok().and_then(|p| {
        fs::read_to_string(&p)
            .ok()
            .map(|s| (p, serde_json::from_str::<Config>(&s).ok()))
    });
    match parsed {
        Some((_, Some(cfg))) => cfg,
        Some((p, None)) => {
            let _ = fs::rename(&p, p.with_extension("json.bad"));
            Config::default()
        }
        None => Config::default(),
    }
}"""),
    # C2: rename_session 统一 CAS + guard（此前只 load 检查，仍有 TOCTOU 窗口）
    ("src-tauri/src/lib.rs",
"""    if let Some(state) = app.try_state::<AppState>() {
        if state.busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("上一条指令还在执行中，请稍候再重命名".into());
        }
        memory::rename_session(&app, &id, &title);
        return Ok(());
    }""",
"""    if let Some(state) = app.try_state::<AppState>() {
        // CAS 占住（与其他会话命令一致：check-then-act 有窗口，且 rename 与
        // agent 收尾的 save_history 并发读改写 sessions.json 会丢更新）
        if state
            .busy
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Err("上一条指令还在执行中，请稍候再重命名".into());
        }
        let _guard = BusyGuard(&state.busy);
        memory::rename_session(&app, &id, &title);
        return Ok(());
    }"""),
    # C3: atomic_write tmp 名唯一化（同文件并发写不再互踩同一临时文件）
    ("src-tauri/src/memory.rs",
"""fn atomic_write(path: &std::path::Path, data: &str) {
    let tmp = path.with_extension("json.tmp");""",
"""fn atomic_write(path: &std::path::Path, data: &str) {
    // tmp 名带纳秒+pid：未串行化的并发写对（如后台总结 vs 手动清记忆）不互踩临时文件
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = path.with_extension(format!("json.{:x}.{}.tmp", nanos, std::process::id()));"""),
    # C4: SSE CRLF 兼容（代理/兼容服务用 \r\n\r\n 分隔时此前一个事件都解析不出且 buf 无限增长）
    ("src-tauri/src/llm.rs",
"""        buf.extend_from_slice(&bytes);

        while let Some(pos) = buf.windows(2).position(|w| w == b"\\n\\n") {""",
"""        buf.extend_from_slice(&bytes);
        // CRLF 归一化：部分代理/兼容服务用 \\r\\n\\r\\n 分隔事件，只匹配 \\n\\n 会永远匹配不上
        // （UTF-8 多字节序列不包含 0x0D，去掉 \\r 不会破坏中文）
        buf.retain(|&b| b != b'\\r');

        while let Some(pos) = buf.windows(2).position(|w| w == b"\\n\\n") {"""),
    # C6: send busy 回填仅当输入框为空（不打扰正在编辑的草稿）
    ("ui/main.js",
"""    if (inputEl.value !== text) { inputEl.value = text; autoGrow(); }""",
"""    if (!inputEl.value) { inputEl.value = text; autoGrow(); } // 输入框为空才回填识别文本，不打扰草稿"""),
    # V1: ensure_server 互斥（warmup 与并发 transcribe 会各拉一个 server：双模型内存 + 孤儿）
    ("src-tauri/src/voice.rs",
"""/// 确保有可用的 whisper-server，返回端口。复用旧实例（含孤儿），失败则启动新的。
async fn ensure_server(cfg: &crate::config::Config) -> Result<u16, String> {
    let model_name = model_file(cfg.voice_model.as_str()).to_string();""",
"""static ENSURING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 确保有可用的 whisper-server，返回端口。复用旧实例（含孤儿），失败则启动新的。
async fn ensure_server(cfg: &crate::config::Config) -> Result<u16, String> {
    // 互斥：「检查+启动」全程串行，防止 warmup 与并发 transcribe 各拉起一个 server
    let _serial = ENSURING.lock().await;
    let model_name = model_file(cfg.voice_model.as_str()).to_string();"""),
    # V3: kill_pid 先验明正身（server.json 残留 pid 可能已被系统复用给无关进程）
    ("src-tauri/src/voice.rs",
"""fn kill_pid(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .creation_flags(0x08000000)
            .output();
    }
}""",
"""fn kill_pid(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 先验明正身：server.json 里的 pid 可能已被系统复用，盲杀会误伤无关进程
        let is_ours = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .creation_flags(0x08000000)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("whisper-server"))
            .unwrap_or(false);
        if !is_ours {
            return;
        }
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .creation_flags(0x08000000)
            .output();
    }
}"""),
    # V4: probe 连接/读写超时（端口被占时此前可能无限阻塞在 async 上下文里）
    ("src-tauri/src/voice.rs",
"""fn probe(port: u16) -> bool {
    let addr = format!("127.0.0.1:{port}");
    std::net::TcpStream::connect(&addr)
        .map(|s| {
            // TCP 通了还不够稳，发一个最小 HTTP 请求确认是我们的 server
            use std::io::{Read, Write};
            let mut s = s;
            let _ = s.write_all(format!("GET / HTTP/1.0\\r\\nHost: {addr}\\r\\n\\r\\n").as_bytes());
            let mut buf = [0u8; 16];
            matches!(s.read(&mut buf), Ok(n) if n > 0)
        })
        .unwrap_or(false)
}""",
"""fn probe(port: u16) -> bool {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;
    let addr = format!("127.0.0.1:{port}");
    let ok: std::net::SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    TcpStream::connect_timeout(&ok, Duration::from_secs(2))
        .map(|mut s| {
            // TCP 通了还不够稳，发一个最小 HTTP 请求确认是我们的 server；
            // 端口被其他程序占用时超时退出，不能在 async 上下文里无限阻塞
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
            let _ = s.write_all(format!("GET / HTTP/1.0\\r\\nHost: {addr}\\r\\n\\r\\n").as_bytes());
            let mut buf = [0u8; 16];
            matches!(s.read(&mut buf), Ok(n) if n > 0)
        })
        .unwrap_or(false)
}"""),
    # V7: wav 临时名原子自增唯一化（pid+subsec_millis 相加在整秒边界会撞名，两条并发识别互相吃录音）
    ("src-tauri/src/voice.rs",
"""    let tmp = std::env::temp_dir().join(format!(
        "vcc_rec_{}.wav",
        std::process::id() as u64 + chrono_millis()
    ));""",
"""    static WAV_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!(
        "vcc_rec_{}_{}.wav",
        std::process::id(),
        WAV_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));"""),
    ("src-tauri/src/voice.rs",
"""fn chrono_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_millis() as u64)
        .unwrap_or(0)
}""",
""""""),
    # T5: 剪贴板打开重试（被输入法/剪贴板管理器短暂占用是高频场景）
    ("src-tauri/src/tools.rs",
"""#[cfg(windows)]
pub fn clipboard_get(text: ())""",
"""#占位不匹配#"""),
]

# clipboard 重试 + GlobalFree 单独处理（需要精确 old，上面最后一项是占位会被跳过）
PATCHES = [p for p in PATCHES if p[1] != "#占位不匹配#"]

PATCHES += [
    ("src-tauri/src/tools.rs",
"""    use windows::Win32::System::Ole::CF_UNICODETEXT;
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
            return Ok("剪贴板里没有文本".into());
        }
        OpenClipboard(None).map_err(|e| format!("打开剪贴板失败: {e}"))?;""",
"""    use windows::Win32::System::Ole::CF_UNICODETEXT;
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
            return Ok("剪贴板里没有文本".into());
        }
        open_clipboard_retry()?;"""),
    ("src-tauri/src/tools.rs",
"""    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    unsafe {
        OpenClipboard(None).map_err(|e| format!("打开剪贴板失败: {e}"))?;""",
"""    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    unsafe {
        open_clipboard_retry()?;"""),
    # clipboard_set 失败路径 GlobalFree（此前每次失败泄漏一块移动内存）
    ("src-tauri/src/tools.rs",
"""            let ptr = GlobalLock(h) as *mut u16;
            if ptr.is_null() {
                return Err("GlobalLock 失败".into());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(h);
            // 成功后系统接管内存，不得 GlobalFree
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0)))
                .map_err(|e| format!("写入剪贴板失败: {e}"))?;
            Ok(())""",
"""            let ptr = GlobalLock(h) as *mut u16;
            if ptr.is_null() {
                let _ = GlobalFree(h);
                return Err("GlobalLock 失败".into());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(h);
            // 失败时系统未接管内存，必须回收防泄漏；成功后系统接管，不得 GlobalFree
            if SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0))).is_err() {
                let _ = GlobalFree(h);
                return Err("写入剪贴板失败".into());
            }
            Ok(())"""),
]

applied, skipped, failed = 0, 0, []
for rel, old, new in PATCHES:
    p = os.path.join(ROOT, rel)
    s = io.open(p, encoding="utf-8").read()
    if new.strip() and new in s:
        skipped += 1
        print(f"[skip] {rel} （已应用）")
        continue
    n = s.count(old)
    if n != 1:
        failed.append(rel)
        print(f"[FAIL] {rel}: old 匹配 {n} 次")
        continue
    io.open(p, "w", encoding="utf-8", newline="").write(s.replace(old, new))
    applied += 1
    print(f"[ok]   {rel} :: {old.splitlines()[0][:50]}")

# 最后：open_clipboard_retry 函数插入（在 clipboard_get 前面）
p = os.path.join(ROOT, "src-tauri/src/tools.rs")
s = io.open(p, encoding="utf-8").read()
helper = """/// 打开剪贴板带重试：输入法/剪贴板管理器/截屏工具短暂占用是高频场景
#[cfg(windows)]
unsafe fn open_clipboard_retry() -> Result<(), String> {
    use windows::Win32::System::DataExchange::OpenClipboard;
    let mut err = String::new();
    for _ in 0..8 {
        match OpenClipboard(None) {
            Ok(()) => return Ok(()),
            Err(e) => err = e.to_string(),
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    Err(format!("打开剪贴板失败（重试 8 次仍被占用）: {err}"))
}

"""
if "open_clipboard_retry" in s and "unsafe fn open_clipboard_retry" in s:
    print("[skip] open_clipboard_retry 已存在")
elif "unsafe fn open_clipboard_retry" not in s:
    anchor = "#[cfg(windows)]\npub fn clipboard_get"
    assert s.count(anchor) == 1, f"anchor x{s.count(anchor)}"
    s = s.replace(anchor, helper + anchor, 1)
    io.open(p, "w", encoding="utf-8", newline="").write(s)
    applied += 1
    print("[ok]   open_clipboard_retry 插入")

print(f"\n总计: 应用 {applied} / 跳过 {skipped} / 失败 {len(failed)}")
sys.exit(1 if failed else 0)
