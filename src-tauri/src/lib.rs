pub mod config;
pub mod llm;
pub mod memory;
pub mod tools;
pub mod voice;

use llm::ChatMessage;
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Listener, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
};

pub static APP_HANDLE: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

pub struct AppState {
    pub history: Mutex<Vec<ChatMessage>>,
    /// agent 并发锁：同一时刻只允许一个 agent 轮次（防历史/流式状态竞争）
    pub busy: std::sync::atomic::AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            history: Mutex::new(Vec::new()),
            busy: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

pub fn run() {
    // WebView2 官方通道：默认背景 = 全透明（alpha 0），从加载第一帧起不画底色。
    // 官方文档确认该环境变量优先级高于 ControllerOptions/Controller API，
    // 兜底 wry 的 PutDefaultBackgroundColor 在部分 runtime 版本上失效的场景。
    std::env::set_var("WEBVIEW2_DEFAULT_BACKGROUND_COLOR", "00FFFFFF");
    tauri::Builder::default()
        .plugin(
            tauri_plugin_single_instance::init(|app, _args, _cwd| {
                // 二次启动 → 呼出已有实例主窗
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                    let _ = app.emit("vcc://invoked", ());
                }
            }),
        )
        .plugin(global_shortcut_plugin())
        .manage(AppState::default())
        .setup(|app| {
            let _ = APP_HANDLE.set(app.handle().clone());
            // 恢复上次对话历史（跨重启的对话流）
            // 必须在窗口创建之前：webview 页面加载后前端立刻 invoke('load_chat_history')，
            // 若晚于 setup_windows 会拿到空历史（实测时序竞争）
            let hist = memory::load_history(app.handle());
            if let Some(state) = app.try_state::<AppState>() {
                if let Ok(mut h) = state.history.lock() {
                    *h = hist;
                }
            }
            setup_windows(app.handle())?;
            setup_tray(app.handle())?;
            // 恢复「演示置顶」偏好
            if config::load(app.handle()).always_on_top {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_always_on_top(true);
                }
            }
            // 基准钩子：VCC_BENCH=1 真机测语音链路耗时（sandbox 里 whisper 推理被 hook 不可跑）
            // 3 轮 syn-5s.wav -> bench.log（首轮含 server 冷启动，后两轮为热态真实延迟）
            if std::env::var("VCC_BENCH").as_deref() == Ok("1") {
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    let rt = tokio::runtime::Runtime::new().expect("bench rt");
                    rt.block_on(async move {
                        use base64::Engine;
                        let wav = match voice::find_tool("tools/tests/syn-5s.wav") {
                            Some(p) => p,
                            None => {
                                eprintln!("bench: syn-5s.wav not found");
                                return;
                            }
                        };
                        let bytes = std::fs::read(&wav).expect("read wav");
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        let mut log = String::new();
                        for i in 1..=3 {
                            let t0 = std::time::Instant::now();
                            let r = voice::transcribe(&b64).await;
                            let dt = t0.elapsed().as_millis();
                            log.push_str(&format!(
                                "run{}: {}ms ok={}\n",
                                i,
                                dt,
                                r.map(|t| t.chars().take(24).collect::<String>())
                                    .unwrap_or_else(|e| format!("ERR {e}"))
                            ));
                            eprintln!("bench run{}: {}ms", i, dt);
                        }
                        let dir = h.path().app_config_dir().expect("cfg dir");
                        let _ = std::fs::write(dir.join("bench.log"), &log);
                        eprintln!("bench done -> bench.log");
                    });
                });
            }
            // server 预热：启动 20s 后后台静默拉起 whisper-server（首次语音识别即热态）
            {
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_secs(20));
                    let rt = match tokio::runtime::Runtime::new() {
                        Ok(r) => r,
                        Err(_) => return,
                    };
                    rt.block_on(async {
                        voice::warmup().await;
                        eprintln!("vcc: whisper server warmed up");
                    });
                });
            }
            // 全局热键：按配置注册（失败回退默认键）
            register_hotkeys(app.handle());
            // server 空闲回收线程（30 分钟无识别自动关闭，省内存）
            voice::start_idle_reaper();
            // E2E 测试钩子：VCC_DEMO=1 时启动即执行一次完整呼出链路
            // （show 主窗 + emit invoked → listening 态 + 全屏光环 + 悬浮窗步骤卡），
            // 不设置则完全无感
            if std::env::var("VCC_DEMO").as_deref() == Ok("1") {
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    if let Some(win) = h.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                    let _ = h.emit("vcc://invoked", ());
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    // 模拟按住麦克风：跑马灯亮起（listening = 真录音态）
                    let _ = h.emit("vcc://phase", serde_json::json!({"phase": "listening"}));
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                    // 悬浮窗演示：直接发 render 事件（真实链路与 agent 执行时一致）
                    let _ = h.emit("vcc://phase", serde_json::json!({"phase": "executing"}));
                    let _ = h.emit("vcc://float", serde_json::json!({
                        "mode": "show",
                        "steps": [
                            {"label": "设置系统音量 → 30%", "status": "done"},
                            {"label": "打开路径 D:\\课件", "status": "done"},
                            {"label": "读取文件夹列表", "status": "running"}
                        ]
                    }));
                    // 步骤全部完成 → done 态（悬浮窗进入 5s 淡出倒计时，链路与真实 agent 收尾一致）
                    std::thread::sleep(std::time::Duration::from_millis(1800));
                    let _ = h.emit("vcc://float", serde_json::json!({
                        "mode": "done",
                        "steps": [
                            {"label": "设置系统音量 → 30%", "status": "done"},
                            {"label": "打开路径 D:\\课件", "status": "done"},
                            {"label": "读取文件夹列表", "status": "done"}
                        ]
                    }));
                    // 主窗同步进入 done 绽放（输入条扫光加速 + 光环 done 阶段）
                    let _ = h.emit("vcc://chat-end", ());
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            agent_run,
            transcribe,
            get_config,
            save_config,
            reset_history,
            load_chat_history,
            get_memory,
            save_memory_cmd,
            hide_floating,
            hide_main,
            set_autostart,
            set_always_on_top,
            voice::probe_env
        ])
        .run(tauri::generate_context!())
        .expect("VCC 启动失败");
}

/* ---------- 全局热键：呼出/隐藏主窗口 ---------- */

fn global_shortcut_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_global_shortcut::ShortcutState;

    // 快捷键在 setup 里按配置动态注册（cfg.hotkey 此前从未生效过）
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                toggle_main(app);
            }
        })
        .build()
}

/// 按配置注册全局热键：自定义键被占用时回退默认键，绝不 panic
fn register_hotkeys(app: &AppHandle) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let cfg = config::load(app);
    let hk = if cfg.hotkey.trim().is_empty() {
        "ctrl+shift+space".to_string()
    } else {
        cfg.hotkey.trim().to_lowercase()
    };
    let gs = app.global_shortcut();
    if let Err(e) = gs.register(hk.as_str()) {
        eprintln!("vcc: hotkey '{hk}' register failed: {e}, fallback to default");
        let _ = gs.register("ctrl+shift+space");
    }
    if hk != "ctrl+alt+k" {
        let _ = gs.register("ctrl+alt+k"); // 备用热键，占用则忽略
    }
}

fn toggle_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
            let _ = app.emit("vcc://invoked", ());
        }
    }
}

/* ---------- 窗口 ---------- */

/// 关闭 Win11 对透明窗口自动套用的系统 backdrop 材质（Mica/Acrylic）。
/// 不关的话，DWM 会把 tao 的 blur-behind 透明通道渲染成「模糊 + 灰调」的
/// 矩形材质层——这就是卡片外那圈淡灰色方框的真正来源。
/// 实测（像素级采样）：设 DWMSBT_NONE 后环区与窗外完全一致（diff = 0）。
#[cfg(windows)]
fn disable_system_backdrop(hwnd: isize) {
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attr: u32,
            value: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
    const DWMSBT_NONE: i32 = 1;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &DWMSBT_NONE as *const i32 as *const std::ffi::c_void,
            4,
        );
    }
}

fn setup_windows(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // 主窗口（对话界面）
    let main = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Voice Control for Class")
        .inner_size(412.0, 672.0)
        .min_inner_size(372.0, 552.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)  // Windows DWM 给透明窗口画的矩形阴影 = 卡片外的灰色方框，必须关
        .resizable(true)
        .visible(false)
        .center()
        .build()?;

    // 右上角悬浮小窗
    let float = WebviewWindowBuilder::new(app, "floating", WebviewUrl::App("floating.html".into()))
        .title("VCC Floating")
        .inner_size(340.0, 96.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;

    // 关闭系统 backdrop 材质（实测：环区灰层 diff 128 -> 0）；
    // v0.1.0 的 apply_acrylic 毛玻璃残留已删除——那是「淡灰色模糊方框」的直接来源，
    // 且与黑白极简卡片体系冲突（卡片自带深色底）
    #[cfg(windows)]
    {
        if let Ok(h) = main.hwnd() {
            disable_system_backdrop(h.0 as isize);
        }
        if let Ok(h) = float.hwnd() {
            disable_system_backdrop(h.0 as isize);
        }
    }

    // 悬浮窗定位到屏幕右上角
    if let Ok(Some(monitor)) = float.current_monitor() {
        let msize = monitor.size();
        let sf = float.scale_factor().unwrap_or(1.0);
        let w = (340.0 * sf) as i32;
        let margin = (18.0 * sf) as i32;
        let x = msize.width as i32 - w - margin;
        let y = margin;
        float.set_position(PhysicalPosition::new(x, y))?;
    }

    // 全屏跑马灯 Overlay：每个显示器一个；透明、置顶、点击穿透、不抢焦点，随 phase 显示/隐藏
    // （诊断开关：VCC_NO_OVERLAY=1 时不创建，用于排查主窗环区灰层来源）
    if std::env::var("VCC_NO_OVERLAY").as_deref() != Ok("1") {
    if let Ok(monitors) = app.available_monitors() {
        for (i, m) in monitors.into_iter().enumerate() {
            let label = if i == 0 { "overlay".to_string() } else { format!("overlay-{i}") };
            let mpos = m.position();
            let msize = m.size();
            let sf = m.scale_factor();
            if let Ok(overlay) = WebviewWindowBuilder::new(
                app,
                label.clone(),
                WebviewUrl::App("overlay.html".into()),
            )
            .title("VCC Overlay")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .focused(false)
            .shadow(false)
            .visible(false)
            .inner_size(msize.width as f64 / sf, msize.height as f64 / sf)
            .build()
            {
                let _ = overlay.set_position(PhysicalPosition::new(mpos.x, mpos.y));
                let _ = overlay.set_ignore_cursor_events(true);
                #[cfg(windows)]
                if let Ok(h) = overlay.hwnd() {
                    disable_system_backdrop(h.0 as isize);
                }
            }
        }
    }
    }

    // phase 事件 → overlay 显示/隐藏。显示集 = { listening(真录音), executing(工具执行) }，
    // 其余（idle/summoned/thinking/done）延迟 950ms 隐藏——给淡出动画留时间，也给
    // 「录音→思考→执行」的短暂 thinking 留缓冲，避免跑马灯闪烁。
    // 代际计数器：每次新 phase 事件都使未决的延迟 hide 失效，
    // 修复「idle 排了 hide → 950ms 内又开始录音 → hide 照样执行把跑马灯误杀」的竞态。
    let h = app.clone();
    let hide_gen = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    app.listen("vcc://phase", move |ev| {
        let phase = serde_json::from_str::<serde_json::Value>(ev.payload())
            .ok()
            .and_then(|v| v["phase"].as_str().map(String::from))
            .unwrap_or_default();
        if phase.is_empty() {
            return;
        }
        let active = phase == "listening" || phase == "executing";
        let gen = hide_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        for (label, w) in h.webview_windows() {
            if !label.starts_with("overlay") {
                continue;
            }
            if active {
                let _ = w.show();
            } else {
                let h2 = h.clone();
                let label2 = label.clone();
                let gen2 = gen;
                let pending = hide_gen.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(950)).await;
                    // 期间若来了任何新 phase 事件（代数已变），放弃本次隐藏
                    if pending.load(std::sync::atomic::Ordering::SeqCst) == gen2 {
                        if let Some(o) = h2.get_webview_window(&label2) {
                            let _ = o.hide();
                        }
                    }
                });
            }
        }
    });

    // 悬浮窗显示：vcc://float mode=show/done → show 窗口（内容渲染在 webview 侧；
    // done 后 5s 由 floating.js 调 hide_floating 收尾）
    let h3 = app.clone();
    app.listen("vcc://float", move |ev| {
        let v = serde_json::from_str::<serde_json::Value>(ev.payload()).unwrap_or_default();
        let mode = v["mode"].as_str().unwrap_or("");
        if mode == "show" || mode == "done" {
            if let Some(w) = h3.get_webview_window("floating") {
                let _ = w.show();
            }
        }
    });

    Ok(())
}

/* ---------- 托盘 ---------- */

fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 VCC", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let tray = TrayIconBuilder::with_id("vcc-tray")
        .icon(app.default_window_icon().expect("缺少应用图标").clone())
        .tooltip("Voice Control for Class")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                    let _ = app.emit("vcc://invoked", ());
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click { button, .. } = event {
                if button == tauri::tray::MouseButton::Left {
                    let app = tray.app_handle();
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.set_focus();
                        let _ = app.emit("vcc://invoked", ());
                    }
                }
            }
        })
        .build(app)?;
    tray.set_visible(true)?;
    Ok(())
}

/* ---------- Tauri 命令 ---------- */

#[tauri::command]
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
}

#[tauri::command]
async fn transcribe(wav_base64: String) -> Result<String, String> {
    voice::transcribe(&wav_base64).await
}

#[tauri::command]
fn get_config(app: AppHandle) -> config::Config {
    config::load(&app)
}

#[tauri::command]
fn save_config(app: AppHandle, config: config::Config) -> Result<(), String> {
    let old_hotkey = config::load(&app).hotkey;
    config::save(&app, &config)?;
    // 热键变更即时生效：解除旧键 → 按新配置重注册（失败自动回退默认键）
    if config.hotkey.trim() != old_hotkey.trim() {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let _ = app.global_shortcut().unregister_all();
        register_hotkeys(&app);
    }
    Ok(())
}

#[tauri::command]
fn reset_history(app: AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut hist) = state.history.lock() {
            // 新对话前：当前会话精华后台并入长期记忆（异步，不阻塞 UI）
            if !hist.is_empty() {
                let msgs = hist.clone();
                let app2 = app.clone();
                tauri::async_runtime::spawn(async move {
                    memory::summarize_into_memory(app2, msgs).await;
                });
            }
            hist.clear();
        }
    }
    memory::clear_history(&app);
}

/// 启动恢复：把持久化历史交给前端渲染（tool 中间轮由前端过滤）
#[tauri::command]
fn load_chat_history(app: AppHandle) -> Vec<ChatMessage> {
    // UI 只渲染最近 40 条，防长对话 DOM 卡顿（AppState 内仍是完整上下文）
    app.state::<AppState>()
        .history
        .lock()
        .map(|h| {
            let start = h.len().saturating_sub(40);
            h[start..].to_vec()
        })
        .unwrap_or_default()
}

#[tauri::command]
fn get_memory(app: AppHandle) -> String {
    memory::load_memory(&app)
}

#[tauri::command]
fn save_memory_cmd(app: AppHandle, summary: String) {
    memory::save_memory(&app, &summary);
}

#[tauri::command]
fn hide_floating(app: AppHandle) {
    if let Some(w) = app.get_webview_window("floating") {
        let _ = w.hide();
    }
}

/* ESC 快速收起主窗（课堂场景一键隐藏；agent 在后台继续跑不中断） */
#[tauri::command]
fn hide_main(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

/* ---------- 开机自启 / 演示置顶 ---------- */

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

fn reg_run(args: &[&str]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg")
        .args(args)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 开机自启：写/删 HKCU Run 键（免管理员；路径含空格以引号包裹）
#[tauri::command]
fn set_autostart(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if on {
        reg_run(&["add", RUN_KEY, "/v", "VCC", "/t", "REG_SZ",
                  "/d", &format!("\"{}\"", exe.display()), "/f"])
    } else {
        reg_run(&["delete", RUN_KEY, "/v", "VCC", "/f"])
    }
}

/// 演示置顶：主窗总是保持在最前（课件/浏览器之上）
#[tauri::command]
fn set_always_on_top(app: AppHandle, on: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_always_on_top(on);
    }
}

