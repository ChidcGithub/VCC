/* ---------- 泵线程：跑马灯/悬浮窗（wry）+ 托盘 + 全局热键 + 单实例呼出 ---------- */
/* 架构：独立线程跑 Win32 消息泵 + PeekMessage 轮询，承载：
   - 每显示器一个全屏 overlay（透明/置顶/点击穿透，HTML=ui/overlay.html 零改动复用）
   - 右上角悬浮窗（floating.html 零改动复用，按钮经 ipc 桥回调）
   - tray-icon 托盘 + global-hotkey 热键
   - phase 代际显示/隐藏（原 lib.rs 逻辑平移）
   JS 兼容：初始化脚本注入 window.__TAURI__ shim（listen/invoke），页面代码无感知 */

use crate::bus::{EventTx, PumpMsg, SharedWinCtl, Step, UiEvent};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use global_hotkey::GlobalHotKeyManager;
use wry::WebView;

/* ---------- JS 桥：__TAURI__ shim + 事件分发 ---------- */

const SHIM: &str = r#"(function(){
  if (window.__TAURI__) return;
  window.__TAURI__ = {
    event: { listen: function(name, cb) {
      window.addEventListener('vcc-evt', function(e){
        if (e.detail && e.detail.name === name) cb({ event: name, id: 0, payload: e.detail.payload });
      });
    }},
    core: { invoke: function(cmd, args) {
      try { window.ipc.postMessage(JSON.stringify({ cmd: cmd, args: (args || {}) })); } catch (e) {}
    }}
  };
  window.__vccEvt = function(name, payloadJson) {
    try {
      var payload = JSON.parse(payloadJson);
      window.dispatchEvent(new CustomEvent('vcc-evt', { detail: { name: name, payload: payload } }));
    } catch (e) {}
  };
})();"#;

/// 生成「向页面分发事件」的 JS（payload 二次编码，杜绝注入）
fn dispatch_js(name: &str, payload: &serde_json::Value) -> String {
    let n = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".into());
    let p = serde_json::to_string(&payload.to_string()).unwrap_or_else(|_| "\"{}\"".into());
    format!("__vccEvt({n},{p});")
}

/* ---------- ui/ 目录静态服务（自定义协议 vcc://ui/…） ---------- */

fn ui_root() -> std::path::PathBuf {
    // dev：exe 向上找仓库 ui/；release：exe 同目录 ui/
    if let Some(p) = crate::voice::find_tool("ui/index.html") {
        return p.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    }
    std::env::current_exe()
        .unwrap_or_default()
        .parent()
        .map(|p| p.join("ui"))
        .unwrap_or_default()
}

fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "woff2" => "font/woff2",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

fn serve_ui(
    _id: wry::WebViewId,
    req: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    let raw = req.uri().path().trim_start_matches('/');
    let rel = if raw.is_empty() { "overlay.html" } else { raw };
    let root = ui_root();
    let full = root.join(rel);
    // 防目录穿越：canonicalize 后必须仍在 ui/ 之内
    let ok = full.canonicalize().ok().map(|c| c.starts_with(&root)).unwrap_or(false);
    let (status, body) = if ok {
        match std::fs::read(&full) {
            Ok(b) => (200, b),
            Err(_) => (404, b"not found".to_vec()),
        }
    } else {
        (404, b"not found".to_vec())
    };
    wry::http::Response::builder()
        .status(status)
        .header("Content-Type", mime_of(rel))
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-cache")
        .body(std::borrow::Cow::Owned(body))
        .unwrap()
}

/* ---------- Win32 ---------- */

#[cfg(windows)]
mod win {
    use windows::core::{w, PCWSTR, BOOL};
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub unsafe fn register_class() {
        use std::sync::OnceLock;
        static ONCE: OnceLock<()> = OnceLock::new();
        if ONCE.set(()).is_err() {
            return;
        }
        let hmodule = GetModuleHandleW(None).expect("GetModuleHandleW");
        let hinstance = HINSTANCE(hmodule.0);
        unsafe extern "system" fn wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            lpszClassName: w!("VCCWeb"),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }

    /// 枚举全部显示器（物理像素矩形）
    pub unsafe fn monitors() -> Vec<RECT> {
        let mut out: Vec<RECT> = Vec::new();
        unsafe extern "system" fn cb(
            _: HMONITOR,
            _: HDC,
            rect: *mut RECT,
            lparam: LPARAM,
        ) -> BOOL {
            let out = &mut *(lparam.0 as *mut Vec<RECT>);
            if !rect.is_null() {
                out.push(*rect);
            }
            BOOL(1)
        }
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(cb),
            LPARAM(&mut out as *mut _ as isize),
        );
        out
    }

    /// 创建无边框透明窗（WS_EX_NOREDIRECTIONBITMAP：无重定向位图，
    /// 从根上避免 tao blur-behind 在 Win11 渲染成灰圈的问题）
    pub unsafe fn create_window(
        ex: WINDOW_EX_STYLE,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        title: &str,
    ) -> Result<HWND, String> {
        let hmodule = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let hinstance = HINSTANCE(hmodule.0);
        let t = wide(title);
        let hwnd = CreateWindowExW(
            ex,
            w!("VCCWeb"),
            PCWSTR(t.as_ptr()),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(hinstance),
            None,
        )
        .map_err(|e| format!("CreateWindowExW 失败: {e}"))?;
        // layered 窗口本体不透明度恒 255（透明来自 WebView2 背景 alpha，不是窗口 alpha）
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        Ok(hwnd)
    }

    pub unsafe fn show(hwnd: isize, activate: bool) {
        if hwnd == 0 {
            return;
        }
        let _ = ShowWindow(
            HWND(hwnd as _),
            if activate { SW_SHOW } else { SW_SHOWNA },
        );
    }

    pub unsafe fn hide(hwnd: isize) {
        if hwnd != 0 {
            let _ = ShowWindow(HWND(hwnd as _), SW_HIDE);
        }
    }

    /// 点击穿透开关（WS_EX_TRANSPARENT 位）
    pub unsafe fn set_click_through(hwnd: isize, on: bool) {
        if hwnd == 0 {
            return;
        }
        let ex = GetWindowLongPtrW(HWND(hwnd as _), GWL_EXSTYLE);
        let next = if on {
            ex | WS_EX_TRANSPARENT.0 as isize
        } else {
            ex & !(WS_EX_TRANSPARENT.0 as isize)
        };
        SetWindowLongPtrW(HWND(hwnd as _), GWL_EXSTYLE, next);
    }

    pub unsafe fn dpi(hwnd: isize) -> u32 {
        if hwnd == 0 {
            96
        } else {
            GetDpiForWindow(HWND(hwnd as _))
        }
    }

    pub unsafe fn post_close(hwnd: isize) {
        if hwnd != 0 {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    pub unsafe fn set_foreground(hwnd: isize) {
        if hwnd != 0 {
            let _ = SetForegroundWindow(HWND(hwnd as _));
        }
    }
}

/// wry 需要的 HasWindowHandle 包装（裸 HWND）
#[cfg(windows)]
struct HwndHost(isize);

#[cfg(windows)]
impl raw_window_handle::HasWindowHandle for HwndHost {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        use raw_window_handle::{RawWindowHandle, Win32WindowHandle, WindowHandle};
        let hwnd = std::num::NonZeroIsize::new(self.0).expect("hwnd 不能为 0");
        let mut w32 = Win32WindowHandle::new(hwnd);
        w32.hinstance = None;
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(w32)) })
    }
}

/* ---------- 托盘 + 热键 ---------- */

const TRAY_PNG: &[u8] = include_bytes!("../icons/32x32.png");

#[cfg(windows)]
fn hotkey_code(tok: &str) -> Option<global_hotkey::hotkey::Code> {
    use global_hotkey::hotkey::Code;
    Some(match tok {
        "space" => Code::Space,
        "up" => Code::ArrowUp,
        "down" => Code::ArrowDown,
        "left" => Code::ArrowLeft,
        "right" => Code::ArrowRight,
        "tab" => Code::Tab,
        "enter" => Code::Enter,
        "esc" | "escape" => Code::Escape,
        "backspace" => Code::Backspace,
        "delete" | "del" => Code::Delete,
        "home" => Code::Home,
        "end" => Code::End,
        "pageup" => Code::PageUp,
        "pagedown" => Code::PageDown,
        "minus" => Code::Minus,
        "equal" => Code::Equal,
        other if other.len() == 1 && other.chars().next().unwrap().is_ascii_alphabetic() => {
            match other.chars().next().unwrap() {
                'a' => Code::KeyA,
                'b' => Code::KeyB,
                'c' => Code::KeyC,
                'd' => Code::KeyD,
                'e' => Code::KeyE,
                'f' => Code::KeyF,
                'g' => Code::KeyG,
                'h' => Code::KeyH,
                'i' => Code::KeyI,
                'j' => Code::KeyJ,
                'k' => Code::KeyK,
                'l' => Code::KeyL,
                'm' => Code::KeyM,
                'n' => Code::KeyN,
                'o' => Code::KeyO,
                'p' => Code::KeyP,
                'q' => Code::KeyQ,
                'r' => Code::KeyR,
                's' => Code::KeyS,
                't' => Code::KeyT,
                'u' => Code::KeyU,
                'v' => Code::KeyV,
                'w' => Code::KeyW,
                'x' => Code::KeyX,
                'y' => Code::KeyY,
                _ => Code::KeyZ,
            }
        }
        other if other.len() == 1 && other.chars().next().unwrap().is_ascii_digit() => {
            match other.chars().next().unwrap() {
                '0' => Code::Digit0,
                '1' => Code::Digit1,
                '2' => Code::Digit2,
                '3' => Code::Digit3,
                '4' => Code::Digit4,
                '5' => Code::Digit5,
                '6' => Code::Digit6,
                '7' => Code::Digit7,
                '8' => Code::Digit8,
                _ => Code::Digit9,
            }
        }
        other if other.len() >= 2
            && other.starts_with('f')
            && other[1..].parse::<u8>().map(|n| (1..=24).contains(&n)).unwrap_or(false) =>
        {
            match other[1..].parse::<u8>().unwrap_or(1) {
                1 => Code::F1,
                2 => Code::F2,
                3 => Code::F3,
                4 => Code::F4,
                5 => Code::F5,
                6 => Code::F6,
                7 => Code::F7,
                8 => Code::F8,
                9 => Code::F9,
                10 => Code::F10,
                11 => Code::F11,
                12 => Code::F12,
                13 => Code::F13,
                14 => Code::F14,
                15 => Code::F15,
                16 => Code::F16,
                17 => Code::F17,
                18 => Code::F18,
                19 => Code::F19,
                20 => Code::F20,
                21 => Code::F21,
                22 => Code::F22,
                23 => Code::F23,
                _ => Code::F24,
            }
        }
        _ => return None,
    })
}

/// 「ctrl+shift+space」→ HotKey；失败回退默认键
#[cfg(windows)]
fn parse_hotkey(s: &str) -> Option<global_hotkey::hotkey::HotKey> {
    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    let mut mods = Modifiers::empty();
    let mut key: Option<Code> = None;
    for t in s.trim().to_lowercase().split('+') {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        match t {
            "ctrl" | "control" | "ctl" => mods |= Modifiers::CONTROL,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" => mods |= Modifiers::ALT,
            "super" | "win" | "meta" | "cmd" => mods |= Modifiers::SUPER,
            _ => key = hotkey_code(t),
        }
    }
    key.map(|k| HotKey::new(Some(mods), k))
}

/* ---------- 主泵 ---------- */

pub fn pump_main(rx: Receiver<PumpMsg>, ev: EventTx, _state: crate::SharedState) {
    #[cfg(windows)]
    unsafe {
        pump_win(rx, ev)
    }
    #[cfg(not(windows))]
    {
        let _ = (rx, ev);
    }
}

#[cfg(windows)]
unsafe fn pump_win(rx: Receiver<PumpMsg>, ev: EventTx) {
    use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::WindowsAndMessaging::{
        MSG, PM_REMOVE, PeekMessageW, TranslateMessage, DispatchMessageW,
    };

    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    win::register_class();
    let ctl = crate::win_ctl();

    // WebView2 共享用户数据目录（同一进程内多 webview 必须共享，否则 profile 锁冲突）
    let mut webctx = wry::WebContext::new(Some(
        crate::config::data_dir().join("webview2"),
    ));

    // 每显示器一个全屏跑马灯
    let mut overlays: Vec<(isize, WebView)> = Vec::new();
    if std::env::var("VCC_NO_OVERLAY").as_deref() != Ok("1") {
        for r in win::monitors() {
            let w = r.right - r.left;
            let h = r.bottom - r.top;
            let ex = windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(
                windows::Win32::UI::WindowsAndMessaging::WS_EX_TOPMOST.0
                    | windows::Win32::UI::WindowsAndMessaging::WS_EX_TOOLWINDOW.0
                    | windows::Win32::UI::WindowsAndMessaging::WS_EX_NOACTIVATE.0
                    | windows::Win32::UI::WindowsAndMessaging::WS_EX_LAYERED.0
                    | windows::Win32::UI::WindowsAndMessaging::WS_EX_NOREDIRECTIONBITMAP.0
                    | windows::Win32::UI::WindowsAndMessaging::WS_EX_TRANSPARENT.0,
            );
            match win::create_window(ex, r.left, r.top, w, h, "VCC Overlay") {
                Ok(hwnd) => {
                    let host = HwndHost(hwnd.0 as isize);
                    match wry::WebViewBuilder::new_with_web_context(&mut webctx)
                        .with_transparent(true)
                        .with_custom_protocol("vcc".into(), serve_ui)
                        .with_initialization_script(SHIM)
                        .with_url("vcc://ui/overlay.html")
                        .build(&host)
                    {
                        Ok(wv) => overlays.push((hwnd.0 as isize, wv)),
                        Err(e) => eprintln!("vcc: overlay webview 失败: {e}"),
                    }
                }
                Err(e) => eprintln!("vcc: overlay 窗口失败: {e}"),
            }
        }
    }
    eprintln!("vcc: {} overlay(s) ready", overlays.len());

    // 右上角悬浮小窗
    let mut floating: Option<(isize, WebView)> = None;
    {
        let mon = win::monitors().first().copied().unwrap_or(windows::Win32::Foundation::RECT {
            left: 0, top: 0, right: 1920, bottom: 1080,
        });
        let ex = windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(
            windows::Win32::UI::WindowsAndMessaging::WS_EX_TOPMOST.0
                | windows::Win32::UI::WindowsAndMessaging::WS_EX_TOOLWINDOW.0
                | windows::Win32::UI::WindowsAndMessaging::WS_EX_NOACTIVATE.0
                | windows::Win32::UI::WindowsAndMessaging::WS_EX_LAYERED.0
                | windows::Win32::UI::WindowsAndMessaging::WS_EX_NOREDIRECTIONBITMAP.0,
        );
        if let Ok(hwnd) =
            win::create_window(ex, mon.left, mon.top, 340, 96, "VCC Floating")
        {
            let hi = hwnd.0 as isize;
            // 按 DPI 缩放并贴右上角
            let s = win::dpi(hi) as f32 / 96.0;
            let w = (340.0 * s) as i32;
            let h = (96.0 * s) as i32;
            let margin = (18.0 * s) as i32;
            let x = mon.right - w - margin;
            let y = mon.top + margin;
            let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
                hwnd,
                Some(windows::Win32::UI::WindowsAndMessaging::HWND_TOPMOST),
                x,
                y,
                w,
                h,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
            );
            let hi_ipc = hi;
            let host = HwndHost(hi);
            match wry::WebViewBuilder::new_with_web_context(&mut webctx)
                .with_transparent(true)
                .with_custom_protocol("vcc".into(), serve_ui)
                .with_initialization_script(SHIM)
                .with_ipc_handler(move |req: wry::http::Request<String>| {
                    // floating.js invoke('hide_floating') → 隐藏悬浮窗
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(req.body()) {
                        if v["cmd"].as_str() == Some("hide_floating") {
                            win::hide(hi_ipc);
                        }
                    }
                })
                .with_url("vcc://ui/floating.html")
                .build(&host)
            {
                Ok(wv) => floating = Some((hi, wv)),
                Err(e) => eprintln!("vcc: floating webview 失败: {e}"),
            }
        }
    }

    // 托盘 + 热键
    let mut tray_hotkey = setup_tray_hotkey();
    if let Err(e) = &tray_hotkey {
        eprintln!("vcc: 托盘/热键初始化失败: {e}");
    }

    let mut msg = MSG::default();
    let mut pending_hide: Option<Instant> = None;

    loop {
        if ctl.exit.load(Ordering::SeqCst) {
            win::post_close(ctl.hwnd()); // 通知 eframe 退出
            break;
        }
        // 单实例呼出请求
        if ctl.show_main.swap(false, Ordering::SeqCst) {
            show_main(&ctl, &ev);
        }
        // 全局热键
        while let Ok(e) = GlobalHotKeyEvent::receiver().try_recv() {
            if e.state() == HotKeyState::Pressed {
                toggle_main(&ctl, &ev);
            }
        }
        // 托盘图标点击
        while let Ok(e) = tray_icon::TrayIconEvent::receiver().try_recv() {
            if let tray_icon::TrayIconEvent::Click { button: tray_icon::MouseButton::Left, .. } = e {
                show_main(&ctl, &ev);
            }
        }
        // 托盘菜单
        while let Ok(e) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            match e.id().0.as_str() {
                "show" => show_main(&ctl, &ev),
                "quit" => ctl.exit.store(true, Ordering::SeqCst),
                _ => {}
            }
        }
        // 后端事件
        while let Ok(m) = rx.try_recv() {
            match m {
                PumpMsg::Phase(p) => {
                    let active = p == "listening" || p == "executing";
                    let js = dispatch_js("vcc://phase", &serde_json::json!({ "phase": p }));
                    for (_, wv) in &overlays {
                        let _ = wv.evaluate_script(&js);
                    }
                    if active {
                        for (h, _) in &overlays {
                            win::show(*h, false);
                        }
                        pending_hide = None;
                    } else {
                        pending_hide = Some(Instant::now() + Duration::from_millis(950));
                    }
                }
                PumpMsg::Float { mode, steps, text } => {
                    let payload = serde_json::json!({
                        "mode": mode,
                        "steps": steps.iter().map(|s: &Step| serde_json::json!({
                            "label": s.label, "status": s.status
                        })).collect::<Vec<_>>(),
                        "text": text,
                    });
                    if let Some((h, wv)) = &floating {
                        if mode == "show" || mode == "done" {
                            win::show(*h, false);
                        }
                        let js = dispatch_js("vcc://float", &payload);
                        let _ = wv.evaluate_script(&js);
                    }
                }
                PumpMsg::Level(l) => {
                    let js = dispatch_js("vcc://level", &serde_json::json!({ "level": l }));
                    for (_, wv) in &overlays {
                        let _ = wv.evaluate_script(&js);
                    }
                }
            }
        }
        // 延迟隐藏（新 phase 会覆盖 pending，天然防误杀）
        if let Some(t) = pending_hide {
            if Instant::now() >= t {
                for (h, _) in &overlays {
                    win::hide(*h);
                }
                pending_hide = None;
            }
        }
        // Win32 消息泵（非阻塞排空）
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        std::thread::sleep(Duration::from_millis(15));
    }

    if let Ok((tray, mgr)) = tray_hotkey {
        let _ = tray.set_visible(false);
        drop(mgr);
    }
}

#[cfg(windows)]
fn show_main(ctl: &SharedWinCtl, ev: &EventTx) {
    let h = ctl.hwnd();
    unsafe {
        win::show(h, true);
        win::set_foreground(h);
    }
    ctl.main_visible.store(true, Ordering::SeqCst);
    ev.send(UiEvent::Invoked);
}

#[cfg(windows)]
fn hide_main(ctl: &SharedWinCtl) {
    unsafe {
        win::hide(ctl.hwnd());
    }
    ctl.main_visible.store(false, Ordering::SeqCst);
}

#[cfg(windows)]
fn toggle_main(ctl: &SharedWinCtl, ev: &EventTx) {
    if ctl.visible() {
        hide_main(ctl);
    } else {
        show_main(ctl, ev);
    }
}

#[cfg(windows)]
fn setup_tray_hotkey() -> Result<(tray_icon::TrayIcon, GlobalHotKeyManager), String> {
    use global_hotkey::GlobalHotKeyManager;
    use tray_icon::TrayIconBuilder;

    let img = image::load_from_memory(TRAY_PNG)
        .map_err(|e| format!("托盘图标解码失败: {e}"))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let icon = tray_icon::Icon::from_rgba(img.into_raw(), w, h)
        .map_err(|e| format!("托盘图标转换失败: {e}"))?;

    let menu = tray_icon::menu::Menu::new();
    let m_show = tray_icon::menu::MenuItem::with_id("show", "显示主窗口", true, None);
    let m_quit = tray_icon::menu::MenuItem::with_id("quit", "退出 VCC", true, None);
    menu
        .append_items(&[&m_show, &m_quit])
        .map_err(|e| format!("托盘菜单失败: {e}"))?;

    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("Voice Control for Class")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(|e| format!("托盘创建失败: {e}"))?;
    let _ = tray.set_visible(true);

    let mgr = GlobalHotKeyManager::new().map_err(|e| format!("热键管理失败: {e}"))?;
    let cfg = crate::config::load();
    let default = parse_hotkey("ctrl+shift+space").expect("默认热键可解析");
    let custom = if cfg.hotkey.trim().is_empty() {
        None
    } else {
        parse_hotkey(&cfg.hotkey)
    };
    let registered = match custom {
        Some(hk) => mgr.register(hk).is_ok(),
        None => false,
    };
    if !registered {
        let _ = mgr.register(default);
        eprintln!("vcc: 热键 '{}' 注册失败，回退默认", cfg.hotkey);
    }
    if let Some(bk) = parse_hotkey("ctrl+alt+k") {
        let _ = mgr.register(bk); // 备用热键，占用则忽略
    }
    Ok((tray, mgr))
}
