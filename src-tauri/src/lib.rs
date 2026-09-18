pub mod bus;
pub mod config;
pub mod llm;
pub mod md;
pub mod memory;
pub mod motion;
pub mod overlay;
pub mod recorder;
pub mod tools;
pub mod ui;
pub mod voice;

use bus::{SharedWinCtl, WinCtl};
use llm::ChatMessage;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

/// 全局共享状态（原 tauri managed AppState 的平替）
pub struct VccState {
    pub history: Mutex<Vec<ChatMessage>>,
    /// 当前会话 id（多会话存储 sessions.json 的 current_id 运行时镜像）
    pub current: Mutex<String>,
    /// agent 并发锁：同一时刻只允许一个 agent 轮次（防历史/流式状态竞争）
    pub busy: AtomicBool,
}

pub type SharedState = Arc<VccState>;

static WIN_CTL: OnceLock<SharedWinCtl> = OnceLock::new();
/// 主窗控制旗标（tools::show_dialog / 泵线程 / UI 线程共用；测试环境安全）
pub fn win_ctl() -> SharedWinCtl {
    WIN_CTL.get_or_init(|| Arc::new(WinCtl::default())).clone()
}

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
fn rt() -> &'static tokio::runtime::Runtime {
    RT.get().expect("tokio runtime 未初始化")
}

/// 测试用 block_on（原 tauri::async_runtime::block_on 平替）
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Runtime::new().expect("test rt").block_on(f)
}

/* ---------- 入口 ---------- */

pub fn run() {
    // WebView2 官方通道：默认背景 = 全透明（alpha 0）——overlay/悬浮窗透明依赖
    std::env::set_var("WEBVIEW2_DEFAULT_BACKGROUND_COLOR", "00FFFFFF");

    #[cfg(windows)]
    dpi_aware();

    // 单实例：端口被占 → 通知已有实例呼出主窗后退出
    let instance_listener =
        match std::net::TcpListener::bind(("127.0.0.1", SINGLE_INSTANCE_PORT)) {
            Ok(l) => l,
            Err(_) => {
                if let Ok(mut s) =
                    std::net::TcpStream::connect(("127.0.0.1", SINGLE_INSTANCE_PORT))
                {
                    let _ = std::io::Write::write_all(&mut s, b"show\n");
                }
                return;
            }
        };

    let (ev_tx, ev_rx) = bus::event_channel();
    let (pump_tx, pump_rx) = bus::pump_channel();
    let ctl = win_ctl();

    // 恢复多会话存储（跨重启的对话流；旧 history.json 自动迁移）
    let (sid, hist) = memory::load_history();
    let state: SharedState = Arc::new(VccState {
        history: Mutex::new(hist),
        current: Mutex::new(sid),
        busy: AtomicBool::new(false),
    });

    // tokio 常驻（agent/SSE/whisper/工具进程）
    let _ = RT.set(tokio::runtime::Runtime::new().expect("tokio runtime"));

    // 二次启动转发线程（单实例监听）
    {
        let ctl2 = ctl.clone();
        std::thread::spawn(move || {
            for stream in instance_listener.incoming() {
                let mut s = match stream {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let mut buf = [0u8; 16];
                if std::io::Read::read(&mut s, &mut buf).is_ok() {
                    ctl2.request_show(); // 泵线程轮询执行 Win32 呼出
                }
            }
        });
    }

    // server 预热：启动 20s 后后台静默拉起 whisper-server（首次语音识别即热态）
    rt().spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        voice::warmup().await;
        eprintln!("vcc: whisper server warmed up");
    });
    // server 空闲回收线程（30 分钟无识别自动关闭，省内存）
    voice::start_idle_reaper();

    // 泵线程：overlay/悬浮窗（wry）+ 托盘 + 全局热键 + 呼出执行
    {
        let ev2 = ev_tx.clone();
        let state2 = state.clone();
        std::thread::spawn(move || overlay::pump_main(pump_rx, ev2, state2));
    }

    // eframe 主循环（阻塞直到窗口关闭/退出）
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Voice Control for Class")
            .with_inner_size([1020.0, 720.0])
            .with_min_inner_size([620.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "VCC",
        native,
        Box::new(move |cc| {
            Ok(Box::new(ui::VccApp::new(cc, state, ev_tx, ev_rx, pump_tx, ctl)))
        }),
    )
    .expect("VCC 启动失败");
}

const SINGLE_INSTANCE_PORT: u16 = 47820;

/// 进程级 Per-Monitor V2 DPI 感知（eframe/winit 也会设，这里提前到泵线程建窗之前，
/// 保证 overlay 用物理像素铺满每个显示器）
#[cfg(windows)]
fn dpi_aware() {
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
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
        // reg 的 stderr 是系统 ACP 编码（中文系统 GBK），按 UTF-8 展示必乱码；
        // 不透传原始输出，给可读中文 + 退出码
        Err(format!(
            "注册表操作失败（reg {} 退出码 {:?}）",
            args.first().copied().unwrap_or("?"),
            out.status.code()
        ))
    }
}

/// 自启动值是否存在（reg query 成功即存在）
fn reg_run_exists() -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg")
        .args(["query", RUN_KEY, "/v", "VCC"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 开机自启：写/删 HKCU Run 键（免管理员；路径含空格以引号包裹）。
/// 删除幂等：值本来就不存在 = 目标已达成（此前直接 delete，值不存在时
/// reg 报「找不到注册表项」把整个保存流程带崩）。
pub fn set_autostart_impl(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if on {
        reg_run(&[
            "add", RUN_KEY, "/v", "VCC", "/t", "REG_SZ",
            "/d", &format!("\"{}\"", exe.display()), "/f",
        ])
    } else {
        if !reg_run_exists() {
            return Ok(());
        }
        reg_run(&["delete", RUN_KEY, "/v", "VCC", "/f"])
    }
}

/// 演示置顶：主窗总是保持在最前（课件/浏览器之上）——UI 设置面板调用
pub fn set_always_on_top_impl(hwnd: isize, on: bool) {
    if hwnd == 0 {
        return;
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, HWND_NOTOPMOST, SET_WINDOW_POS_FLAGS,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE,
        };
        let top = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
        let _ = SetWindowPos(
            windows::Win32::Foundation::HWND(hwnd as *mut _),
            Some(top),
            0, 0, 0, 0,
            SET_WINDOW_POS_FLAGS(SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOACTIVATE.0),
        );
    }
}
