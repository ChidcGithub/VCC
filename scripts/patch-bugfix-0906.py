# -*- coding: utf-8 -*-
"""2026-09-06 bug 修复批量补丁（子代理审查结论落地）
每个补丁断言 old 唯一；new 已存在则幂等跳过。"""
import io, sys, os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PATCHES = [
    # ============ lib.rs ============
    # R1: agent_run 直接 await（invoke resolve = agent 真正结束）+ BusyGuard + 错误路径补 chat-end
    ("src-tauri/src/lib.rs",
"""#[tauri::command]
async fn agent_run(app: AppHandle, text: String) -> Result<(), String> {
    // 同步抢占：拒绝并发轮次（前端已拦一层，这里兜底）
    if let Some(state) = app.try_state::<AppState>() {
        if state.busy.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let _ = app.emit(
                "vcc://chat",
                serde_json::json!({"role": "err", "text": "上一条指令还在执行中，请稍候"}),
            );
            return Ok(());
        }
    }
    tauri::async_runtime::spawn(async move {
        let result = llm::run_agent(&app, text).await;
        if let Err(e) = result {
            let _ = app.emit("vcc://chat", serde_json::json!({"role": "err", "text": e}));
            let _ = app.emit("vcc://phase", serde_json::json!({"phase": "idle"}));
            let _ = app.emit(
                "vcc://float",
                serde_json::json!({"mode": "done", "steps": [], "text": "出错了，详情见主窗口"}),
            );
        }
        if let Some(state) = app.try_state::<AppState>() {
            state.busy.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    });
    Ok(())
}""",
"""/// busy 占位守卫：作用域结束（含提前 return / panic）自动释放，杜绝「占住不放」
struct BusyGuard<'a>(&'a std::sync::atomic::AtomicBool);
impl std::ops::Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

#[tauri::command]
async fn agent_run(app: AppHandle, text: String) -> Result<(), String> {
    // 同步抢占：拒绝并发轮次（前端已拦一层，这里兜底）
    let state = app.try_state::<AppState>().ok_or("状态未就绪")?;
    if state.busy.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let _ = app.emit(
            "vcc://chat",
            serde_json::json!({"role": "err", "text": "上一条指令还在执行中，请稍候"}),
        );
        return Ok(());
    }
    let _guard = BusyGuard(&state.busy);
    // 直接 await（此前 spawn 后立即返回，invoke 几毫秒就 resolve，
    // 前端 finally/agentBusy 门禁/会话刷新全部拿到错误时序）
    let result = llm::run_agent(&app, text).await;
    if let Err(e) = result {
        let _ = app.emit("vcc://chat", serde_json::json!({"role": "err", "text": e}));
        // 补发流结束信号：错误路径也复位前端 streamBubble / 完成绽放
        let _ = app.emit("vcc://chat-end", serde_json::json!({}));
        let _ = app.emit("vcc://phase", serde_json::json!({"phase": "idle"}));
        let _ = app.emit(
            "vcc://float",
            serde_json::json!({"mode": "done", "steps": [], "text": "出错了，详情见主窗口"}),
        );
    }
    Ok(())
}"""),
    # R2: reset_history CAS 占住 busy（check-then-act 有竞态窗口）
    ("src-tauri/src/lib.rs",
"""    if let Some(state) = app.try_state::<AppState>() {
        if state.busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("上一条指令还在执行中".into());
        }
        let hist = state.history.lock().map(|h| h.clone()).unwrap_or_default();""",
"""    if let Some(state) = app.try_state::<AppState>() {
        // CAS 占住 busy（只 load 检查存在竞态窗口：检查通过后 agent 可立刻插队 push 消息）
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
            return Err("上一条指令还在执行中".into());
        }
        let _guard = BusyGuard(&state.busy);
        let hist = state.history.lock().map(|h| h.clone()).unwrap_or_default();"""),
    # R3: switch_session CAS
    ("src-tauri/src/lib.rs",
"""        if state.busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("上一条指令还在执行中，请稍候再切换".into());
        }
        // 写回当前会话""",
"""        if state
            .busy
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Err("上一条指令还在执行中，请稍候再切换".into());
        }
        let _guard = BusyGuard(&state.busy);
        // 写回当前会话"""),
    # R4: delete_session CAS
    ("src-tauri/src/lib.rs",
"""        if state.busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("上一条指令还在执行中，请稍候再删除".into());
        }""",
"""        if state
            .busy
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Err("上一条指令还在执行中，请稍候再删除".into());
        }
        let _guard = BusyGuard(&state.busy);"""),
    # R5: clear_all_sessions CAS
    ("src-tauri/src/lib.rs",
"""        if state.busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("上一条指令还在执行中".into());
        }
        if let Ok(mut h) = state.history.lock() {
            h.clear();
        }
        memory::clear_all_sessions(&app);""",
"""        if state
            .busy
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return Err("上一条指令还在执行中".into());
        }
        let _guard = BusyGuard(&state.busy);
        if let Ok(mut h) = state.history.lock() {
            h.clear();
        }
        memory::clear_all_sessions(&app);"""),
    # R6: rename_session 补 busy 检查（此前唯一漏网，与 save_history 读改写竞争）
    ("src-tauri/src/lib.rs",
"""/// 重命名会话
#[tauri::command]
fn rename_session(app: AppHandle, id: String, title: String) -> Result<(), String> {
    memory::rename_session(&app, &id, &title);
    Ok(())
}""",
"""/// 重命名会话（busy 时拒绝：rename 与 agent 收尾的 save_history 并发读改写 sessions.json 会丢更新）
#[tauri::command]
fn rename_session(app: AppHandle, id: String, title: String) -> Result<(), String> {
    if let Some(state) = app.try_state::<AppState>() {
        if state.busy.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("上一条指令还在执行中，请稍候再重命名".into());
        }
        memory::rename_session(&app, &id, &title);
        return Ok(());
    }
    Err("状态未就绪".into())
}"""),
    # ============ llm.rs ============
    # L1: SSE 字节缓冲（中文跨 TCP 包不再乱码）
    ("src-tauri/src/llm.rs",
"""    let mut stream = resp.bytes_stream();
    let mut buf = String::new();""",
"""    let mut stream = resp.bytes_stream();
    // 字节缓冲：多字节 UTF-8（中文）被 TCP 分包切在 chunk 边界时，
    // 逐 chunk from_utf8_lossy 会产生 U+FFFD 永久乱码——事件边界处整段解码才安全
    let mut buf: Vec<u8> = Vec::new();"""),
    ("src-tauri/src/llm.rs",
"""    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| format!("流中断: {e}"))?;
        buf.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(pos) = buf.find("\\n\\n") {
            let event: String = buf.drain(..pos + 2).collect();
            for line in event.lines() {""",
"""    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| format!("流中断: {e}"))?;
        buf.extend_from_slice(&bytes);

        while let Some(pos) = buf.windows(2).position(|w| w == b"\\n\\n") {
            let drained: Vec<u8> = buf.drain(..pos + 2).collect();
            let event = String::from_utf8_lossy(&drained).into_owned();
            for line in event.lines() {"""),
    # L2: chat-end 从流函数移除（每轮都发 → 前端把中间工具轮误判为回答结束）
    ("src-tauri/src/llm.rs",
"""    // 冲刷剩余 delta（流尾不足 33ms 的尾巴）
    if !delta_buf.is_empty() {
        let _ = app.emit("vcc://chat-delta", json!({"text": delta_buf}));
    }
    let _ = app.emit("vcc://chat-end", json!({}));""",
"""    // 冲刷剩余 delta（流尾不足 33ms 的尾巴）
    if !delta_buf.is_empty() {
        let _ = app.emit("vcc://chat-delta", json!({"text": delta_buf}));
    }
    // chat-end 不在这里发：流函数每轮调用，每轮都发会让前端把中间工具轮
    // 误判为回答结束（phase 闪 done→idle 抖动）；最终轮由 run_agent 统一发，
    // 失败路径由 lib.rs agent_run 补发。"""),
    # L3: 上下文裁剪保护 tool 配对（孤立 tool 消息会让 API 持续 400，会话报废）
    ("src-tauri/src/llm.rs",
"""            let dropped: Vec<ChatMessage> = hist.drain(..l - 60).collect();
            if !dropped.is_empty() {""",
"""            let dropped: Vec<ChatMessage> = hist.drain(..l - 60).collect();
            // 裁剪不得切断 assistant(tool_calls) 与 role=tool 的配对：
            // 保留区开头若是孤立 tool 消息，继续前吞到配对边界，否则请求持续 400
            while hist.first().map(|m| m.role == "tool").unwrap_or(false) {
                hist.remove(0);
            }
            if !dropped.is_empty() {"""),
    # L4: run_agent 主体包 async block + 失败回滚 user 消息
    ("src-tauri/src/llm.rs",
"""    let mut steps: Vec<Value> = Vec::new();
    let mut rounds = 0;

    loop {
        rounds += 1;
        if rounds > 10 {
            return Err("这个任务步骤太多了，试试拆成两步告诉我".into());
        }""",
"""    let mut steps: Vec<Value> = Vec::new();
    let mut rounds = 0;

    // 主体包进 async block：失败路径统一走回滚（重试不产生重复 user 轮次）
    let result: Result<(), String> = async {
    loop {
        rounds += 1;
        if rounds > 10 {
            return Err("这个任务步骤太多了，试试拆成两步告诉我".into());
        }"""),
    ("src-tauri/src/llm.rs",
"""        // 纯工具调用无文本输出时，主窗口补一个完成气泡
        if answer.trim().is_empty() || answer == "（已完成）" {
            let _ = app.emit("vcc://chat", json!({"role": "ai", "text": "✓ 已执行完成"}));
        }
        let _ = app.emit("vcc://float", json!({"mode": "done", "steps": steps, "text": answer}));
        // done（而非直接 idle）：主窗完成绽放 + 跑马灯淡出；900ms 后由前端统一回落 idle
        let _ = app.emit("vcc://phase", json!({"phase": "done"}));
        return Ok(());
    }
}""",
"""        // 纯工具调用无文本输出时，主窗口补一个完成气泡
        if answer.trim().is_empty() || answer == "（已完成）" {
            let _ = app.emit("vcc://chat", json!({"role": "ai", "text": "✓ 已执行完成"}));
        }
        // 最终流结束信号：仅整次回答结束发一次（前端渲染 markdown + TTS + 完成绽放）
        let _ = app.emit("vcc://chat-end", json!({}));
        let _ = app.emit("vcc://float", json!({"mode": "done", "steps": steps, "text": answer}));
        // done（而非直接 idle）：主窗完成绽放 + 跑马灯淡出；900ms 后由前端统一回落 idle
        let _ = app.emit("vcc://phase", json!({"phase": "done"}));
        return Ok(());
    }
    };

    if result.is_err() {
        // 回滚：把本次 push 的 user 消息弹出并落盘（API 失败/流中断不留残轮，
        // 且防止重试后历史里出现两条相同指令）
        if let Some(state) = app.try_state::<crate::AppState>() {
            if let Ok(mut hist) = state.history.lock() {
                let is_ours = hist
                    .last()
                    .map(|m| m.role == "user" && m.content.as_deref() == Some(text.as_str()))
                    .unwrap_or(false);
                if is_ours {
                    hist.pop();
                    crate::memory::save_history(app, &hist);
                }
            }
        }
    }
    result
}"""),
    # ============ memory.rs ============
    # M1: atomic_write helper（放 persist_sessions_file 前）
    ("src-tauri/src/memory.rs",
"""fn persist_sessions_file(app: &AppHandle, file: &SessionsFile) {
    if let Ok(path) = sessions_path(app) {
        if let Ok(json) = serde_json::to_string_pretty(file) {
            let _ = fs::write(path, json);
        }
    }
}""",
"""/// 原子写：临时文件 + rename 替换（进程中途被杀/断电不留半截 JSON）
fn atomic_write(path: &std::path::Path, data: &str) {
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, data).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

fn persist_sessions_file(app: &AppHandle, file: &SessionsFile) {
    if let Ok(path) = sessions_path(app) {
        if let Ok(json) = serde_json::to_string_pretty(file) {
            atomic_write(&path, &json);
        }
    }
}"""),
    # M2: 损坏 sessions.json 保留 .bad 现场（此前直接用空文件覆盖 = 全部会话不可逆丢失）
    ("src-tauri/src/memory.rs",
"""    if let Ok(path) = sessions_path(app) {
        if let Ok(s) = fs::read_to_string(&path) {
            if let Ok(f) = serde_json::from_str::<SessionsFile>(&s) {
                return f;
            }
        }
    }""",
"""    if let Ok(path) = sessions_path(app) {
        if let Ok(s) = fs::read_to_string(&path) {
            match serde_json::from_str::<SessionsFile>(&s) {
                Ok(f) => return f,
                Err(_) => {
                    // 损坏文件保留现场（.bad）供人工恢复，绝不用空文件覆盖
                    let _ = fs::rename(&path, path.with_extension("json.bad"));
                }
            }
        }
    }"""),
    # M3: save_memory 原子写
    ("src-tauri/src/memory.rs",
"""    if let Ok(json) = serde_json::to_string_pretty(&file) {
        let _ = fs::write(path, json);
    }
}

/* ---------- AI 记忆总结（后台异步，失败静默） ---------- */""",
"""    if let Ok(json) = serde_json::to_string_pretty(&file) {
        atomic_write(&path, &json);
    }
}

/* ---------- AI 记忆总结（后台异步，失败静默） ---------- */"""),
    # M4: summarize 单飞（并发触发时后写者会抹掉先写者的成果）
    ("src-tauri/src/memory.rs",
"""/// 后台总结入口：永不 panic、失败静默——记忆是增强功能，绝不干扰主对话流程。
pub async fn summarize_into_memory(app: AppHandle, msgs: Vec<ChatMessage>) {
    if let Err(_e) = summarize_inner(&app, &msgs).await {
        // 静默：网络故障/无 Key 时不弹错、不打断
    }
}""",
"""static SUMMARIZING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 后台总结入口：永不 panic、失败静默——记忆是增强功能，绝不干扰主对话流程。
pub async fn summarize_into_memory(app: AppHandle, msgs: Vec<ChatMessage>) {
    // 单飞：总结耗时可达分钟级，并发触发时都读同一份旧记忆再整文件覆盖，
    // 后写者会抹掉先写者的成果——后到的直接跳过
    if SUMMARIZING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let _ = summarize_inner(&app, &msgs).await; // 静默：网络故障/无 Key 时不弹错
    SUMMARIZING.store(false, std::sync::atomic::Ordering::SeqCst);
}"""),
    # ============ config.rs ============
    # C1: config.json 原子写 + 损坏保留 .bad（否则下次 save 用默认覆盖，API Key 静默丢失）
    ("src-tauri/src/config.rs",
"""        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}""",
"""        .and_then(|p| fs::read_to_string(p).ok())
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
        Some((p, Some(cfg))) => cfg,
        Some((p, None)) => {
            let _ = fs::rename(&p, p.with_extension("json.bad"));
            Config::default()
        }
        None => Config::default(),
    }
}"""),
    ("src-tauri/src/config.rs",
"""pub fn save(app: &AppHandle, cfg: &Config) -> Result<(), String> {
    let path = config_path(app)?;
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}""",
"""pub fn save(app: &AppHandle, cfg: &Config) -> Result<(), String> {
    let path = config_path(app)?;
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    // 原子写：临时文件 + rename 替换（进程中途被杀不留半截 JSON，
    // 半截 JSON 会让下次 load 回落默认配置、API Key 被静默抹掉）
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}"""),
    # ============ tools.rs ============
    # T1: 黑名单扩容（EncodedCommand/FromBase64String 是黑名单绕过的主通道）
    ("src-tauri/src/tools.rs",
"""    "iex(", "|iex", "| iex", "start-process -verb runas",
];""",
"""    "iex(", "|iex", "| iex", "start-process -verb runas",
    // 编码执行类：powershell -EncodedCommand <b64> 的命令体是 Base64，
    // 明文匹配全部失效，必须连编码通道一起拦
    "encodedcommand", " -enc ", " -ec ", "frombase64string", "|enc",
];"""),
    # T2: .exe 归一化（reg.exe add / shutdown.exe 绕过 "reg add" 黑名单）
    ("src-tauri/src/tools.rs",
"""pub fn is_blocked(command: &str) -> Option<&'static str> {
    let lower = command.to_lowercase();""",
"""pub fn is_blocked(command: &str) -> Option<&'static str> {
    // .exe 归一化：reg.exe add / shutdown.exe 这类写法此前绕过黑名单
    let lower = command.to_lowercase().replace(".exe", "");"""),
    # T3: powershell 超时杀子进程（kill_on_drop：不留孤儿继续执行）
    ("src-tauri/src/tools.rs",
"""    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output())""",
"""    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // 超时放弃 output() future 时同步杀掉子进程（默认 kill_on_drop=false 会留孤儿继续跑）
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output())"""),
    # T4: OCR 脚本同样处理
    ("src-tauri/src/tools.rs",
"""        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }
    let out = tokio::time::timeout(Duration::from_secs(40), cmd.output())""",
"""        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true); // 超时孤儿防护，同 powershell_raw
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }
    let out = tokio::time::timeout(Duration::from_secs(40), cmd.output())"""),
    # T5: 兜底超时文案与事实对齐（此前说「已自动关闭」但弹窗其实还挂着）
    ("src-tauri/src/tools.rs",
"""        0 => format!("弹窗 {} 秒内未得到响应，已自动关闭", p.timeout_secs),""",
"""        0 => format!(
            "弹窗 {} 秒内未得到响应，可能仍停留在屏幕上，请提示用户手动关闭",
            p.timeout_secs
        ),"""),
    # ============ ui/main.js ============
    # F1: send busy 分支不吞语音识别文本
    ("ui/main.js",
"""  if (agentBusy) {
    // 静默吞输入会让人困惑：状态行轻提示 1.5s，输入内容保留
    phaseEl.textContent = '上一条还在执行中…';""",
"""  if (agentBusy) {
    // 不吞输入：语音识别出的文本放回输入条（文字输入时本来就还在），稍后可手动发
    if (inputEl.value !== text) { inputEl.value = text; autoGrow(); }
    // 状态行轻提示 1.5s
    phaseEl.textContent = '上一条还在执行中…';"""),
    # F2: 录音竞态（双击泄漏流 / 授权等待期松手丢停止事件）
    ("ui/main.js",
"""async function startRecording() {
  if (recorder) return; // 防双击/重复 pointerdown 泄漏麦克风流
  try {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    });
    const ctx = new AudioContext();""",
"""let recWanting = false; // getUserMedia 授权等待期的意图标志：期间双击/松手都不泄漏流
async function startRecording() {
  if (recorder || recWanting) return; // 防双击/重复 pointerdown 泄漏麦克风流
  recWanting = true;
  try {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    });
    if (!recWanting) {
      // 授权等待期间已松手：立即释放流，不进入录音
      stream.getTracks().forEach((t) => t.stop());
      return;
    }
    const ctx = new AudioContext();"""),
    ("ui/main.js",
"""    pump();
  } catch (e) {
    addErrorBubble('无法访问麦克风：' + e, { noRetry: true });
  }
}""",
"""    pump();
  } catch (e) {
    addErrorBubble('无法访问麦克风：' + e, { noRetry: true });
  } finally {
    recWanting = false;
  }
}"""),
    ("ui/main.js",
"""async function stopRecording() {
  if (!recorder) return;
  const r = recorder;""",
"""async function stopRecording() {
  if (!recorder) { recWanting = false; return; } // 授权等待期松手：撤销意图，流拿到即释放
  const r = recorder;"""),
    # F3: 新对话 reset 失败不清 UI（此前把正在流式输出的气泡全部清掉）
    ("ui/main.js",
"""    try {
      currentSid = await invoke('reset_history');
    } catch (e) {
      addErrorBubble(String(e), { noRetry: true });
    }
    renderHistory([], false);""",
"""    try {
      currentSid = await invoke('reset_history');
    } catch (e) {
      // 后端拒绝（busy）时保留当前对话：清掉正在流式输出的气泡再让 delta 重长会错乱
      addErrorBubble(String(e), { noRetry: true });
      chatEl.classList.remove('clearing');
      delete chatEl.dataset.clearing;
      return;
    }
    renderHistory([], false);"""),
    # F4: renderHistory 重置重试锚点（重试按钮不得跨会话重发旧指令）
    ("ui/main.js",
"""function renderHistory(msgs, withDivider) {
  chatEl.innerHTML = '';
  runningTools = {};
  streamBubble = null;""",
"""function renderHistory(msgs, withDivider) {
  chatEl.innerHTML = '';
  runningTools = {};
  streamBubble = null;
  lastUserText = ''; // 重试按钮不得跨会话重发旧指令
  for (const k of Object.keys(toolStarts)) delete toolStarts[k];"""),
    # F5: 保存设置后空态恢复走常驻引用 esTemplate（getElementById 拿不到已摘除节点）
    ("ui/main.js",
"""    const esNow = document.getElementById('empty-state');
    if (esNow) renderChips(esNow, currentCmds.length ? currentCmds : DEFAULT_CMDS);
    // 配完 Key 回到界面：若无任何消息则把空态请回来（新手闭环）
    if (!document.querySelector('#chat .bubble')) {
      const es2 = document.getElementById('empty-state');
      if (es2) {
        renderChips(es2, currentCmds.length ? currentCmds : DEFAULT_CMDS);
        es2.classList.remove('hidden');
      }
    }""",
"""    // 空态恢复必须走常驻引用 esTemplate：节点可能已从 DOM 摘除，getElementById 拿不到
    if (!document.querySelector('#chat .bubble') && esTemplate) {
      if (!esTemplate.isConnected) chatEl.appendChild(esTemplate);
      esTemplate.classList.remove('hidden');
      renderChips(esTemplate, currentCmds.length ? currentCmds : DEFAULT_CMDS);
    }"""),
    # F6: 删除 vcc://transcript 死监听（Rust 端从未发射）
    ("ui/main.js",
"""listen('vcc://transcript', (e) => {
  showTranscript(e.payload.text);
});

/* AI 长期记忆后台更新完成（上下文压缩 / 新对话归档时触发） */""",
"""/* AI 长期记忆后台更新完成（上下文压缩 / 新对话归档时触发） */"""),
    # F7: probe_env 代际保护（旧慢请求后到覆盖新状态）
    ("ui/main.js",
"""    vs.textContent = '识别服务：检测中…';
    invoke('probe_env').then((info) => {
      const m = /端口 (\\d+)/.exec(String(info));
      vs.textContent = m && m[1] !== '0'
        ? '识别服务：运行中（端口 ' + m[1] + '）'
        : '识别服务：待命（首次语音时自动拉起）';
    }).catch(() => {
      vs.textContent = '识别服务：环境缺失（未找到 tools/whisper）';
    });""",
"""    vs.textContent = '识别服务：检测中…';
    const seq = (vs.dataset.seq = String(Number(vs.dataset.seq || 0) + 1));
    invoke('probe_env').then((info) => {
      if (vs.dataset.seq !== seq) return; // 旧请求后到，让位给新一轮检测
      const m = /端口 (\\d+)/.exec(String(info));
      vs.textContent = m && m[1] !== '0'
        ? '识别服务：运行中（端口 ' + m[1] + '）'
        : '识别服务：待命（首次语音时自动拉起）';
    }).catch(() => {
      if (vs.dataset.seq === seq) vs.textContent = '识别服务：环境缺失（未找到 tools/whisper）';
    });"""),
    # F8: applyTheme 写 localStorage（供 index.html 首帧预读）
    ("ui/main.js",
"""function applyTheme(t) {
  const dark = t !== 'light';
  document.body.classList.toggle('dark', dark);
  document.body.classList.toggle('light', !dark);""",
"""function applyTheme(t) {
  const dark = t !== 'light';
  document.body.classList.toggle('dark', dark);
  document.body.classList.toggle('light', !dark);
  try { localStorage.setItem('vcc-theme', dark ? 'dark' : 'light'); } catch (_) {}"""),
    # F9: 删 preview 死类（CSS 里从未定义）
    ("ui/main.js",
"""  if (demo) {
    document.body.classList.add('preview');
    addBubble('user', '把音量调到 30，然后打开 D 盘的课件文件夹');""",
"""  if (demo) {
    addBubble('user', '把音量调到 30，然后打开 D 盘的课件文件夹');"""),
    # ============ ui/index.html ============
    # H1: 浅色用户首帧预读（此前先渲染一帧深色再跳浅色）
    ("ui/index.html",
"""  <title>VCC</title>
  <link rel="stylesheet" href="icons.css" />
  <link rel="stylesheet" href="style.css" />""",
"""  <title>VCC</title>
  <script>
    // 主题预读：浅色用户首帧即为 light（main.js 的 applyTheme 要等异步 get_config，
    // 直接跳变会闪一帧深色；localStorage 由 applyTheme 同步维护）
    try {
      if (localStorage.getItem('vcc-theme') === 'light') {
        document.addEventListener('DOMContentLoaded', function () {
          document.body.classList.remove('dark');
          document.body.classList.add('light');
        });
      }
    } catch (e) {}
  </script>
  <link rel="stylesheet" href="icons.css" />
  <link rel="stylesheet" href="style.css" />"""),
    # ============ ui/floating.js ============
    # FL1: 内层 hide 计时器跟踪（淡出窗口期内新任务 show 被误杀）
    ("ui/floating.js",
"""let fadeTimer = null;""",
"""let fadeTimer = null;
let hideTimer = null;"""),
    ("ui/floating.js",
"""function render(payload) {
  clearTimeout(fadeTimer);
  card.classList.remove('fade');""",
"""function render(payload) {
  clearTimeout(fadeTimer);
  clearTimeout(hideTimer); // 内层 hide 也要撤：淡出窗口期内新任务 show 会被 450ms 后的隐藏误杀
  card.classList.remove('fade');"""),
    ("ui/floating.js",
"""      fadeTimer = setTimeout(async () => {
        card.classList.add('fade');
        setTimeout(() => invoke('hide_floating'), 450);
      }, 5000);""",
"""      fadeTimer = setTimeout(async () => {
        card.classList.add('fade');
        hideTimer = setTimeout(() => invoke('hide_floating'), 450);
      }, 5000);"""),
    # ============ ui/floating.html ============
    # FH1: icons.css 重复引入（补丁脚本插入时与已有行叠加）
    ("ui/floating.html",
"""  <link rel="stylesheet" href="icons.css" />
  <link rel="stylesheet" href="icons.css" />
  <link rel="stylesheet" href="floating.css" />""",
"""  <link rel="stylesheet" href="icons.css" />
  <link rel="stylesheet" href="floating.css" />"""),
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
        failed.append((rel, old[:60].replace("\n", "\\n"), n))
        print(f"[FAIL] {rel}: old 匹配 {n} 次")
        continue
    io.open(p, "w", encoding="utf-8", newline="").write(s.replace(old, new))
    applied += 1
    print(f"[ok]   {rel}")

print(f"\n总计: 应用 {applied} / 跳过 {skipped} / 失败 {len(failed)}")
sys.exit(1 if failed else 0)
