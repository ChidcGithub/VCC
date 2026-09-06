use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use serde_json::{json, Value};
use std::time::Duration;

type ToolResult = Result<String, String>;

pub async fn execute(name: &str, args: &str) -> ToolResult {
    let v: Value = serde_json::from_str(args).unwrap_or_else(|_| json!({}));
    match name {
        "set_volume" => set_volume(num(&v, "level")?),
        "adjust_volume" => adjust_volume(num(&v, "delta")?),
        "get_volume" => get_volume(),
        "toggle_mute" => toggle_mute(),
        "set_brightness" => set_brightness(num(&v, "level")?).await,
        "mouse_move" => mouse_move(int(&v, "x")?, int(&v, "y")?),
        "mouse_click" => mouse_click(&v),
        "mouse_drag" => mouse_drag(
            int(&v, "from_x")?,
            int(&v, "from_y")?,
            int(&v, "to_x")?,
            int(&v, "to_y")?,
        ),
        "scroll_wheel" => scroll_wheel(&v),
        "type_text" => type_text(str(&v, "text")?),
        "press_hotkey" => press_hotkey(str(&v, "keys")?),
        "run_command" => run_command(str(&v, "command")?).await,
        "open_path" => open_path(str(&v, "path")?).await,
        "open_app" => open_app(str(&v, "name")?).await,
        "list_dir" => list_dir(str(&v, "path")?),
        "read_file" => read_file(&v),
        "write_file" => write_file(&v),
        "search_files" => search_files(str(&v, "dir")?, str(&v, "pattern")?),
        "screenshot" => screenshot().await,
        "read_screen" => read_screen(v.get("window").and_then(|s| s.as_str()).unwrap_or("")).await,
        "ocr_screen" => ocr_screen(v.get("window").and_then(|s| s.as_str()).unwrap_or("")).await,
        "show_dialog" => show_dialog(&v).await,
        "clipboard" => match v.get("action").and_then(|x| x.as_str()) {
            Some("set") => {
                #[cfg(windows)]
                {
                    clipboard_set(str(&v, "text")?)
                }
                #[cfg(not(windows))]
                {
                    Err("仅支持 Windows".into())
                }
            }
            _ => {
                #[cfg(windows)]
                {
                    clipboard_get()
                }
                #[cfg(not(windows))]
                {
                    Err("仅支持 Windows".into())
                }
            }
        },
        _ => Err(format!("未知工具: {name}")),
    }
}

/* ================= 参数辅助 ================= */

fn num(v: &Value, k: &str) -> Result<f64, String> {
    v.get(k)
        .and_then(|x| x.as_f64())
        .ok_or(format!("缺少数值参数: {k}"))
}
fn int(v: &Value, k: &str) -> Result<i32, String> {
    v.get(k)
        .and_then(|x| x.as_i64())
        .map(|x| x as i32)
        .ok_or(format!("缺少整数参数: {k}"))
}
fn str<'a>(v: &'a Value, k: &str) -> Result<&'a str, String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .ok_or(format!("缺少字符串参数: {k}"))
}

/* ================= 音量（Core Audio） ================= */

#[cfg(windows)]
fn endpoint_volume() -> Result<windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume, String>
{
    use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED};

    unsafe {
        // 线程可能尚未初始化 COM；已初始化或模式不同都会返回错误，忽略即可
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .map_err(|e| e.to_string())?;
        device
            .Activate::<windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume>(
                CLSCTX_ALL,
                None,
            )
            .map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
pub fn set_volume(level: f64) -> ToolResult {
    let v = (level as f32).clamp(0.0, 100.0) / 100.0;
    unsafe {
        let vol = endpoint_volume()?;
        vol.SetMasterVolumeLevelScalar(v, std::ptr::null())
            .map_err(|e| e.to_string())?;
    }
    Ok(format!("音量已设置为 {}%", v * 100.0))
}

#[cfg(not(windows))]
pub fn set_volume(_level: f64) -> ToolResult {
    Err("仅支持 Windows".into())
}

#[cfg(windows)]
pub fn get_volume() -> ToolResult {
    unsafe {
        let vol = endpoint_volume()?;
        let v = vol.GetMasterVolumeLevelScalar().map_err(|e| e.to_string())?;
        let muted = vol.GetMute().map_err(|e| e.to_string())?;
        Ok(format!("当前音量 {}%，静音: {}", (v * 100.0).round(), muted.as_bool()))
    }
}

#[cfg(not(windows))]
pub fn get_volume() -> ToolResult {
    Err("仅支持 Windows".into())
}

/// 相对调节：「大点声」= +15，「小点声」= -15。读当前 → 加 delta → clamp，一轮搞定
#[cfg(windows)]
pub fn adjust_volume(delta: f64) -> ToolResult {
    unsafe {
        let vol = endpoint_volume()?;
        // 静音状态下「大点声」却没声音会很困惑：调音量即解除静音
        if vol.GetMute().map(|m| m.as_bool()).unwrap_or(false) {
            let _ = vol.SetMute(false, std::ptr::null());
        }
        let cur = vol.GetMasterVolumeLevelScalar().map_err(|e| e.to_string())?;
        let next = ((cur as f64 * 100.0) + delta).clamp(0.0, 100.0);
        vol.SetMasterVolumeLevelScalar((next / 100.0) as f32, std::ptr::null())
            .map_err(|e| e.to_string())?;
        Ok(format!("音量已调到 {}%", next.round()))
    }
}

#[cfg(not(windows))]
pub fn adjust_volume(_delta: f64) -> ToolResult {
    Err("仅支持 Windows".into())
}

#[cfg(windows)]
pub fn toggle_mute() -> ToolResult {
    unsafe {
        let vol = endpoint_volume()?;
        let muted = vol.GetMute().map_err(|e| e.to_string())?;
        let next = !muted.as_bool();
        vol.SetMute(next, std::ptr::null()).map_err(|e| e.to_string())?;
        Ok(if next { "已静音".into() } else { "已取消静音".into() })
    }
}

#[cfg(not(windows))]
pub fn toggle_mute() -> ToolResult {
    Err("仅支持 Windows".into())
}

/* ================= 亮度（WMI） ================= */

pub async fn set_brightness(level: f64) -> ToolResult {
    let l = (level as u32).clamp(0, 100);
    let script = format!(
        "Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightnessMethods | Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{{Timeout=0; Brightness={l}}}"
    );
    match powershell_raw(&script).await {
        Ok(_) => Ok(format!("亮度已设置为 {l}%")),
        Err(_) => Err("亮度调节失败（该功能仅笔记本内置屏幕支持）".into()),
    }
}

async fn powershell_raw(script: &str) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // 超时放弃 output() future 时同步杀掉子进程（默认 kill_on_drop=false 会留孤儿继续跑）
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .map_err(|_| "执行超时".to_string())?
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/* ================= 截屏（System.Drawing，虚拟屏全捕） ================= */

/// 全屏截图（覆盖所有显示器的虚拟屏）→ 保存 %TEMP% → 用系统看图器打开
pub async fn screenshot() -> ToolResult {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("vcc-screenshot-{ts}.png"));
    let p = path.display().to_string().replace("'", "''");
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms,System.Drawing;          $vs=[System.Windows.Forms.SystemInformation]::VirtualScreen;          $b=New-Object System.Drawing.Bitmap $vs.Width, $vs.Height;          $g=[System.Drawing.Graphics]::FromImage($b);          $g.CopyFromScreen($vs.X, $vs.Y, 0, 0, $b.Size);          $g.Dispose(); $b.Save('{p}'); $b.Dispose(); Write-Output 'ok'",
        p = p
    );
    powershell_raw(&script).await?;
    open_path(&path.display().to_string()).await?;
    Ok(format!("截图已保存并打开: {}", path.display()))
}

/* ================= 鼠标 / 键盘（enigo） ================= */

pub fn new_enigo() -> Result<Enigo, String> {
    Enigo::new(&Settings::default()).map_err(|e| e.to_string())
}

fn mouse_move(x: i32, y: i32) -> ToolResult {
    let mut en = new_enigo()?;
    en.move_mouse(x, y, Coordinate::Abs).map_err(|e| e.to_string())?;
    Ok(format!("鼠标已移动到 ({x}, {y})"))
}

fn mouse_click(v: &Value) -> ToolResult {
    let mut en = new_enigo()?;
    if let (Some(x), Some(y)) = (v.get("x").and_then(|x| x.as_i64()), v.get("y").and_then(|y| y.as_i64())) {
        en.move_mouse(x as i32, y as i32, Coordinate::Abs)
            .map_err(|e| e.to_string())?;
    }
    let double = v.get("double").and_then(|d| d.as_bool()).unwrap_or(false);
    let button = if v.get("button").and_then(|b| b.as_str()) == Some("right") {
        Button::Right
    } else {
        Button::Left
    };
    if double {
        en.button(button, Direction::Click).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(60));
        en.button(button, Direction::Click).map_err(|e| e.to_string())?;
        Ok("双击完成".into())
    } else {
        en.button(button, Direction::Click).map_err(|e| e.to_string())?;
        Ok("点击完成".into())
    }
}

fn mouse_drag(fx: i32, fy: i32, tx: i32, ty: i32) -> ToolResult {
    let mut en = new_enigo()?;
    en.move_mouse(fx, fy, Coordinate::Abs).map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_millis(120));
    en.button(Button::Left, Direction::Press).map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_millis(90));
    // 分段移动，模拟真实拖动
    let steps = 12;
    for i in 1..=steps {
        let x = fx + (tx - fx) * i / steps;
        let y = fy + (ty - fy) * i / steps;
        en.move_mouse(x, y, Coordinate::Abs).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(25));
    }
    std::thread::sleep(Duration::from_millis(90));
    en.button(Button::Left, Direction::Release).map_err(|e| e.to_string())?;
    Ok(format!("已拖动 ({fx},{fy}) → ({tx},{ty})"))
}

fn scroll_wheel(v: &Value) -> ToolResult {
    let dir = v.get("direction").and_then(|d| d.as_str()).unwrap_or("down");
    let clicks = v.get("clicks").and_then(|c| c.as_i64()).unwrap_or(3).clamp(1, 20) as i32;
    let mut en = new_enigo()?;
    let delta = if dir == "up" { 120 * clicks } else { -120 * clicks };
    en.scroll(delta, enigo::Axis::Vertical).map_err(|e| e.to_string())?;
    Ok(format!("已向{}滚动 {} 格", if dir == "up" { "上" } else { "下" }, clicks))
}

fn type_text(text: &str) -> ToolResult {
    let mut en = new_enigo()?;
    en.text(text).map_err(|e| e.to_string())?;
    Ok(format!("已输入 {} 个字符", text.chars().count()))
}

fn parse_key(token: &str) -> Option<Key> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(Key::Control),
        "shift" => Some(Key::Shift),
        "alt" => Some(Key::Alt),
        "win" | "meta" | "cmd" => Some(Key::Meta),
        "enter" | "return" => Some(Key::Return),
        "esc" | "escape" => Some(Key::Escape),
        "tab" => Some(Key::Tab),
        "space" => Some(Key::Space),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        "up" => Some(Key::UpArrow),
        "down" => Some(Key::DownArrow),
        "left" => Some(Key::LeftArrow),
        "right" => Some(Key::RightArrow),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" => Some(Key::PageUp),
        "pagedown" => Some(Key::PageDown),
        t if t.len() == 2 && t.starts_with('f') => {
            let n: u32 = t[1..].parse().ok()?;
            if !(1..=12).contains(&n) {
                return None;
            }
            Some(match n {
                1 => Key::F1, 2 => Key::F2, 3 => Key::F3, 4 => Key::F4,
                5 => Key::F5, 6 => Key::F6, 7 => Key::F7, 8 => Key::F8,
                9 => Key::F9, 10 => Key::F10, 11 => Key::F11, _ => Key::F12,
            })
        }
        t if t.chars().count() == 1 => t.chars().next().map(Key::Unicode),
        _ => None,
    }
}

fn press_hotkey(keys: &str) -> ToolResult {
    let parts: Vec<Key> = keys
        .split(['+', ' '])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| parse_key(s).ok_or(format!("无法识别按键: {s}")))
        .collect::<Result<_, _>>()?;
    if parts.is_empty() {
        return Err("组合键为空".into());
    }
    let mut en = new_enigo()?;
    for k in &parts {
        en.key(*k, Direction::Press).map_err(|e| e.to_string())?;
    }
    for k in parts.iter().rev() {
        en.key(*k, Direction::Release).map_err(|e| e.to_string())?;
    }
    Ok(format!("已按下 {keys}"))
}

/* ================= OCR（Windows.Media.Ocr 像素级文字识别） ================= */
/* UIA 读不到图片 / canvas / 自绘界面里的字；OCR 直接识别像素。中文语言包优先。 */

#[cfg(windows)]
fn virtual_screen_rect() -> (i32, i32, i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

#[cfg(windows)]
fn window_rect_by_title(kw: &str) -> Result<(i32, i32, i32, i32), String> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, ShowWindow, SW_RESTORE};
    let kw_lower = kw.to_lowercase();
    let (_, hwnd) = list_top_windows()
        .into_iter()
        .find(|(t, _)| t.to_lowercase().contains(&kw_lower))
        .ok_or_else(|| format!("未找到标题含「{kw}」的窗口（可用 ocr_screen 不带参数先识别全屏）"))?;
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            std::thread::sleep(std::time::Duration::from_millis(450));
        }
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).map_err(|e| format!("取窗口区域失败: {e}"))?;
        Ok((r.left, r.top, r.right - r.left, r.bottom - r.top))
    }
}

/// OCR 脚本（PowerShell 运行时调 WinRT。不用 windows crate 的 WinRT 绑定：
/// 它会静态链接 api-ms-win-core-winrt-*, 部分环境 loader 解析不了导致进程 0xC0000139）
static OCR_PS1: &[u8] = include_bytes!("../../scripts/ocr-screen.ps1");

async fn powershell_file(script_path: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("powershell");
    cmd.arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(script_path)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true); // 超时孤儿防护，同 powershell_raw
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }
    let out = tokio::time::timeout(Duration::from_secs(40), cmd.output())
        .await
        .map_err(|_| "OCR 超时（40s）".to_string())?
        .map_err(|e| format!("脚本启动失败: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub async fn ocr_screen(window_kw: &str) -> ToolResult {
    let kw = window_kw.trim().to_string();
    // 窗口定位 / 还原涉及 Win32 调用，放阻塞线程
    let rect = tokio::task::spawn_blocking(move || -> Result<(i32, i32, i32, i32), String> {
        Ok(if kw.is_empty() {
            virtual_screen_rect()
        } else {
            window_rect_by_title(&kw)?
        })
    })
    .await
    .map_err(|e| format!("ocr_screen 调度失败: {e}"))??;
    let (x, y, w, h) = rect;

    // 嵌入脚本落盘（覆盖写，保证与二进制版本一致）
    let ps1 = std::env::temp_dir().join("vcc-ocr-screen.ps1");
    std::fs::write(&ps1, OCR_PS1).map_err(|e| format!("写 OCR 脚本失败: {e}"))?;

    let out = powershell_file(
        &ps1.display().to_string(),
        &[
            "-X", &x.to_string(),
            "-Y", &y.to_string(),
            "-W", &w.to_string(),
            "-H", &h.to_string(),
        ],
    )
    .await?;

    if out == "__NO_OCR__" {
        return Err("系统没有可用的 OCR 语言包（设置 → 时间和语言 → 语言和区域 → 添加中文并勾选基本键入）".into());
    }
    let text = out.trim().to_string();
    if text.is_empty() {
        return Ok("OCR 完成，但没有识别到文字。".into());
    }
    let lines = text.lines().count();
    let shown: String = text.chars().take(1200).collect();
    Ok(format!("OCR（{w}x{h}px，{lines} 行）：\n{shown}"))
}

/* ================= AI 自定义弹窗（Win32 原生 TaskDialog，回传用户选择） ================= */

/// 弹窗参数（模型 JSON → 结构化，含截断/缺省/校验）
#[derive(Clone, Debug)]
pub struct DialogParams {
    pub title: String,
    pub body: String,
    pub buttons: Vec<String>,
    pub default_idx: usize,
    pub warn_icon: bool,
    pub info_icon: bool,
    pub timeout_secs: u64,
}

pub fn dialog_params(v: &Value) -> Result<DialogParams, String> {
    let take = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    let title = take(
        v.get("title").and_then(|x| x.as_str()).unwrap_or("提示").trim(),
        40,
    );
    let body = take(
        v.get("body").and_then(|x| x.as_str()).unwrap_or("").trim(),
        600,
    );
    if body.is_empty() {
        return Err("缺少 body（弹窗正文）".into());
    }
    let mut buttons: Vec<String> = Vec::new();
    let mut has_primary = false;
    let mut has_danger = false;
    let mut primary_idx: Option<usize> = None;
    if let Some(arr) = v.get("buttons").and_then(|x| x.as_array()) {
        for b in arr.iter().take(4) {
            let label = b
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| b.get("label").and_then(|x| x.as_str()).map(|s| s.to_string()));
            let Some(label) = label else { continue };
            if label.trim().is_empty() {
                continue;
            }
            match b.get("style").and_then(|x| x.as_str()).unwrap_or("normal") {
                "primary" => {
                    has_primary = true;
                    if primary_idx.is_none() {
                        primary_idx = Some(buttons.len());
                    }
                }
                "danger" => has_danger = true,
                _ => {}
            }
            buttons.push(take(label.trim(), 12));
        }
    }
    if buttons.is_empty() {
        buttons.push("好".into());
        has_primary = true;
    }
    Ok(DialogParams {
        title,
        body,
        default_idx: primary_idx.unwrap_or(0),
        warn_icon: has_danger,
        info_icon: !has_danger && has_primary,
        buttons,
        timeout_secs: v
            .get("timeout_secs")
            .and_then(|x| x.as_u64())
            .unwrap_or(120)
            .clamp(10, 600),
    })
}

#[cfg(windows)]
mod task_dialog {
    use windows::core::{HSTRING, PCWSTR, HRESULT};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::Win32::UI::Controls::{
        TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOG_NOTIFICATIONS,
        TD_INFORMATION_ICON, TD_WARNING_ICON, TDF_ALLOW_DIALOG_CANCELLATION,
        TDF_CALLBACK_TIMER, TDF_POSITION_RELATIVE_TO_WINDOW, TDN_TIMER,
    };
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

    /// TDM_CLOSE = WM_USER + 102（wParam=0 → TaskDialogIndirect 返回 0，与用户 Esc 的 IDCANCEL 区分开）
    const TDM_CLOSE: u32 = 0x0400 + 102;

    type TaskDialogIndirectFn = unsafe extern "system" fn(
        pctd: *const TASKDIALOGCONFIG,
        pnbutton: *mut i32,
        pnradiobutton: *mut i32,
        pfverificationflagchecked: *mut windows::core::BOOL,
    ) -> HRESULT;

    /// TDF_CALLBACK_TIMER 下 TDN_TIMER 的 lParam = 已流逝毫秒；到点发 TDM_CLOSE(0)
    unsafe extern "system" fn dialog_cb(
        hwnd: HWND,
        msg: TASKDIALOG_NOTIFICATIONS,
        _w: WPARAM,
        lparam: LPARAM,
        timeout_ms: isize,
    ) -> HRESULT {
        if msg == TDN_TIMER && lparam.0 >= timeout_ms {
            let _ = SendMessageW(hwnd, TDM_CLOSE, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        HRESULT(0)
    }

    /// 动态解析 comctl32 v6 的 TaskDialogIndirect（零静态 import：lib 测试 exe 的
    /// comctl32 静态导入在受限环境 loader 会 0xC0000139，动态加载彻底绕开）
    fn resolve_task_dialog() -> Result<TaskDialogIndirectFn, String> {
        unsafe {
            let lib = LoadLibraryW(windows::core::w!("comctl32.dll"))
                .map_err(|e| format!("加载 comctl32 失败: {e}"))?;
            let proc = GetProcAddress(lib, windows::core::s!("TaskDialogIndirect"))
                .ok_or("系统缺 comctl32 v6（TaskDialog 不可用）")?;
            Ok(std::mem::transmute::<_, TaskDialogIndirectFn>(proc))
        }
    }

    /// 阻塞显示原生弹窗，返回 TaskDialogIndirect 的原始按钮 id：
    /// >=1001 = 自定义按钮（1001+下标）；2(IDCANCEL) = 用户 Esc/系统关闭；0 = 超时
    pub unsafe fn show(
        parent: isize,
        title: &str,
        body: &str,
        buttons: &[String],
        default_idx: usize,
        warn_icon: bool,
        info_icon: bool,
        timeout_ms: u64,
    ) -> Result<i32, String> {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let task_dialog_indirect = resolve_task_dialog()?;

        let wtitle = HSTRING::from("VCC");
        let wmain = HSTRING::from(title);
        let wbody = HSTRING::from(body);
        let wlabels: Vec<HSTRING> = buttons.iter().map(|s| HSTRING::from(s.as_str())).collect();
        let tdbuttons: Vec<TASKDIALOG_BUTTON> = wlabels
            .iter()
            .enumerate()
            .map(|(i, h)| TASKDIALOG_BUTTON {
                nButtonID: 1001 + i as i32,
                pszButtonText: PCWSTR::from_raw(h.as_ptr()),
            })
            .collect();

        let icon = if warn_icon {
            TD_WARNING_ICON
        } else if info_icon {
            TD_INFORMATION_ICON
        } else {
            PCWSTR::null()
        };

        let cfg = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: HWND(parent as *mut core::ffi::c_void),
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION
                | TDF_CALLBACK_TIMER
                | TDF_POSITION_RELATIVE_TO_WINDOW,
            pszWindowTitle: PCWSTR::from_raw(wtitle.as_ptr()),
            Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: icon },
            pszMainInstruction: PCWSTR::from_raw(wmain.as_ptr()),
            pszContent: PCWSTR::from_raw(wbody.as_ptr()),
            cButtons: tdbuttons.len() as u32,
            pButtons: tdbuttons.as_ptr(),
            nDefaultButton: 1001 + default_idx as i32,
            pfCallback: Some(dialog_cb),
            lpCallbackData: timeout_ms as isize,
            ..Default::default()
        };

        let mut clicked: i32 = 0;
        let mut radio: i32 = 0;
        let mut verified = windows::core::BOOL::default();
        let hr = task_dialog_indirect(&cfg, &mut clicked, &mut radio, &mut verified);
        if hr.is_err() {
            return Err(format!("TaskDialog 失败: {hr}"));
        }
        Ok(clicked)
    }
}

#[cfg(not(windows))]
mod task_dialog {
    pub unsafe fn show(
        _parent: isize, _title: &str, _body: &str, _buttons: &[String],
        _default_idx: usize, _warn: bool, _info: bool, _timeout_ms: u64,
    ) -> Result<i32, String> {
        Err("非 Windows 平台暂不支持弹窗".into())
    }
}

async fn show_dialog(v: &Value) -> ToolResult {
    use tauri::Manager;
    let p = dialog_params(v)?;
    // 父窗口：主窗可见时弹窗贴合其上（HWND 以 isize 跨线程传递）
    let parent = crate::APP_HANDLE
        .get()
        .and_then(|app| app.get_webview_window("main"))
        .filter(|w| w.is_visible().unwrap_or(false))
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
        .unwrap_or(0);

    let (tx, rx) = std::sync::mpsc::channel::<Result<i32, String>>();
    let (title, body, buttons) = (p.title.clone(), p.body.clone(), p.buttons.clone());
    let (di, wi, ii, tms) = (p.default_idx, p.warn_icon, p.info_icon, p.timeout_secs * 1000);
    std::thread::spawn(move || {
        // 专用线程 + STA COM（任务对话框要求 COM 初始化；不占用 tokio 阻塞池线程）
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            task_dialog::show(parent, &title, &body, &buttons, di, wi, ii, tms)
        }));
        let _ = tx.send(r.unwrap_or_else(|_| Err("弹窗线程崩溃".into())));
    });

    // 硬上限：正常路径由弹窗自身计时器先关（TDM_CLOSE），这里兜底防线程挂死
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(p.timeout_secs + 15),
        tokio::task::spawn_blocking(move || rx.recv()),
    )
    .await;
    let clicked = match outcome {
        Ok(Ok(Ok(Ok(id)))) => id,
        _ => 0, // 通道断/挂死 → 按超时处理
    };

    Ok(match clicked {
        id if id >= 1001 => format!(
            "用户选择了「{}」",
            p.buttons.get((id - 1001) as usize).cloned().unwrap_or_else(|| "?".into())
        ),
        2 => "用户按 Esc 关闭了弹窗（未选择）".into(),
        0 => format!(
            "弹窗 {} 秒内未得到响应，可能仍停留在屏幕上，请提示用户手动关闭",
            p.timeout_secs
        ),
        _ => "弹窗已关闭（未选择）".into(),
    })
}

/* ================= 剪贴板 ================= */

/// 打开剪贴板带重试：输入法/剪贴板管理器/截屏工具短暂占用是高频场景
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

#[cfg(windows)]
pub fn clipboard_get() -> ToolResult {
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
            return Ok("剪贴板里没有文本".into());
        }
        open_clipboard_retry()?;
        let inner = (|| -> Result<String, String> {
            let h = GetClipboardData(CF_UNICODETEXT.0 as u32)
                .map_err(|e| format!("读剪贴板失败: {e}"))?;
            let ptr = GlobalLock(windows::Win32::Foundation::HGLOBAL(h.0)) as *const u16;
            if ptr.is_null() {
                return Err("GlobalLock 失败".into());
            }
            // 以 NUL 定长，2MB 上限防异常数据
            let mut len = 0usize;
            while len < 1_000_000 && *ptr.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(h.0));
            Ok(s)
        })();
        let _ = CloseClipboard();
        let s = inner?;
        let total = s.chars().count();
        let shown: String = s.chars().take(800).collect();
        Ok(if total > 800 {
            format!("剪贴板文本（共 {total} 字，前 800）：\n{shown}")
        } else {
            format!("剪贴板文本：\n{s}")
        })
    }
}

#[cfg(windows)]
pub fn clipboard_set(text: &str) -> ToolResult {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, SetClipboardData,
    };
    use windows::Win32::Foundation::GlobalFree;
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    unsafe {
        open_clipboard_retry()?;
        let inner = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| format!("清空剪贴板失败: {e}"))?;
            let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)
                .map_err(|e| format!("内存分配失败: {e}"))?;
            let ptr = GlobalLock(h) as *mut u16;
            if ptr.is_null() {
                let _ = GlobalFree(Some(h));
                return Err("GlobalLock 失败".into());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            let _ = GlobalUnlock(h);
            // 失败时系统未接管内存，必须回收防泄漏；成功后系统接管，不得 GlobalFree
            if SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0))).is_err() {
                let _ = GlobalFree(Some(h));
                return Err("写入剪贴板失败".into());
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        inner?;
        Ok(format!("已复制到剪贴板（{} 字符）", text.chars().count()))
    }
}

/* ================= 屏幕读取（UIA 无障碍树 → 结构化文本） ================= */
/* DeepSeek 是纯文本模型，看不了截图；把前台窗口的可交互元素（名称/角色/中心坐标）
   dump 成文本，配合 mouse_click/type_text/press_hotkey 即可"看见并操作"任意应用。 */

pub async fn read_screen(window: &str) -> ToolResult {
    let w = window.trim().to_string();
    // UIA/COM 在阻塞线程跑，不占 async 池
    tokio::task::spawn_blocking(move || uia_dump(&w))
        .await
        .map_err(|e| format!("read_screen 调度失败: {e}"))?
}

#[cfg(windows)]
pub fn uia_dump(window: &str) -> ToolResult {
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsIconic, ShowWindow, SW_RESTORE};

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("UIA 初始化失败: {e}"))?;

        // 无参数：前台窗口；前台是 VCC 自己则列出可见窗口
        if window.is_empty() || window == "前台" {
            let hwnd = GetForegroundWindow();
            if is_own_window(hwnd) {
                return top_window_list("当前前台是 VCC 自己。可见窗口：");
            }
            return dump_element(&automation, hwnd);
        }
        // all：列出全部可见顶层窗口
        if window == "all" || window == "全部" {
            return top_window_list("可见窗口列表（用 read_screen window=标题关键字 查看内容）：");
        }
        // 按标题关键字找窗口
        let lower = window.to_lowercase();
        let wins = list_top_windows();
        if let Some((_title, hwnd)) = wins
            .iter()
            .find(|(t, h)| !h.0.is_null() && !is_own_window(*h) && t.to_lowercase().contains(&lower))
        {
            if IsIconic(*hwnd).as_bool() {
                let _ = ShowWindow(*hwnd, SW_RESTORE);
                std::thread::sleep(Duration::from_millis(350));
            }
            dump_element(&automation, *hwnd)
        } else {
            let titles: Vec<String> = wins.iter().take(12).map(|(t, _)| format!("「{t}」")).collect();
            Err(format!("未找到含「{window}」的可见窗口。当前窗口：{}", titles.join(" ")))
        }
    }
}

/// dump 一个窗口的 UIA 控件树：可交互元素按阅读顺序（先上后左）编号
#[cfg(windows)]
fn dump_element(automation: &windows::Win32::UI::Accessibility::IUIAutomation, hwnd: windows::Win32::Foundation::HWND) -> ToolResult {
    use windows::Win32::UI::Accessibility::{IUIAutomationElement, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId, UIA_ComboBoxControlTypeId, UIA_DataItemControlTypeId, UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_HyperlinkControlTypeId, UIA_ListItemControlTypeId, UIA_MenuItemControlTypeId, UIA_RadioButtonControlTypeId, UIA_SliderControlTypeId, UIA_SpinnerControlTypeId, UIA_SplitButtonControlTypeId, UIA_TabItemControlTypeId, UIA_TreeItemControlTypeId};

    unsafe {
        let root = automation
            .ElementFromHandle(hwnd)
            .map_err(|_| "窗口句柄失效（窗口可能已关闭）".to_string())?;
        let title = clean_text(&root.CurrentName().unwrap_or_default().to_string(), 30);
        let walker = automation
            .ControlViewWalker()
            .map_err(|e| format!("UIA walker 获取失败: {e}"))?;

        const CLICKABLE: &[i32] = &[
            UIA_ButtonControlTypeId.0, UIA_MenuItemControlTypeId.0, UIA_TabItemControlTypeId.0,
            UIA_CheckBoxControlTypeId.0, UIA_RadioButtonControlTypeId.0, UIA_ComboBoxControlTypeId.0,
            UIA_HyperlinkControlTypeId.0, UIA_ListItemControlTypeId.0, UIA_TreeItemControlTypeId.0,
            UIA_DataItemControlTypeId.0, UIA_SliderControlTypeId.0, UIA_SpinnerControlTypeId.0,
            UIA_SplitButtonControlTypeId.0, UIA_EditControlTypeId.0, UIA_DocumentControlTypeId.0,
        ];

        let mut found: Vec<(i32, i32, String)> = Vec::new();
        let mut stack: Vec<(IUIAutomationElement, u32)> = vec![(root, 0)];
        let mut visited = 0usize;
        while let Some((el, depth)) = stack.pop() {
            visited += 1;
            if visited > 2000 || found.len() >= 44 {
                break;
            }
            let name_b = el.CurrentName().unwrap_or_default();
            let ct = el.CurrentControlType().unwrap_or(UIA_ButtonControlTypeId);
            let offscreen = el.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(true);
            if !offscreen {
                if let Ok(r) = el.CurrentBoundingRectangle() {
                    let w = r.right - r.left;
                    let h = r.bottom - r.top;
                    if w > 0 && h > 0 {
                        let name = clean_text(&name_b.to_string(), 24);
                        let role = role_name(ct.0);
                        let clickable = CLICKABLE.contains(&ct.0);
                        let is_self_title = !name.is_empty() && name == title;
                        if (clickable || !name.is_empty()) && !is_self_title {
                            found.push((
                                r.top + h / 2,
                                r.left + w / 2,
                                format!("[{}] {} \"{}\" @({},{})", found.len() + 1, role, name, r.left + w / 2, r.top + h / 2),
                            ));
                        }
                    }
                }
            }
            if depth < 10 {
                // 收集子元素，倒序入栈保证先遍历靠前的
                let mut kids: Vec<IUIAutomationElement> = Vec::new();
                if let Ok(mut child) = walker.GetFirstChildElement(&el) {
                    loop {
                        kids.push(child.clone());
                        match walker.GetNextSiblingElement(&child) {
                            Ok(next) => child = next,
                            Err(_) => break,
                        }
                    }
                }
                for k in kids.into_iter().rev() {
                    stack.push((k, depth + 1));
                }
            }
        }

        if found.is_empty() {
            return Ok(format!("窗口「{title}」没有读到可交互元素（可能是特殊渲染窗口，试试 run_command）。"));
        }
        // 阅读顺序：先上后左
        found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut out = format!("窗口「{title}」可交互元素 {} 个（@即中心坐标，用 mouse_click x,y 点击）：\n", found.len());
        for (_, _, line) in found.iter().take(44) {
            out.push_str(line);
            out.push('\n');
        }
        Ok(out)
    }
}

#[cfg(windows)]
fn top_window_list(prefix: &str) -> ToolResult {
    let wins = list_top_windows();
    if wins.is_empty() {
        return Ok(format!("{prefix}（无）"));
    }
    let lines: Vec<String> = wins
        .iter()
        .take(15)
        .map(|(t, _)| format!("- 「{t}」"))
        .collect();
    Ok(format!(
        "{prefix}\n{}\n提示：用 read_screen window=标题关键字 查看某窗口的可点击元素。",
        lines.join("\n")
    ))
}

#[cfg(windows)]
fn list_top_windows() -> Vec<(String, windows::Win32::Foundation::HWND)> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowLongPtrW, GetWindowThreadProcessId, GWL_EXSTYLE, WS_EX_TOOLWINDOW};
    use std::cell::RefCell;

    thread_local! {
        static OUT: RefCell<Vec<(String, HWND)>> = const { RefCell::new(Vec::new()) };
    }
    unsafe extern "system" fn enum_proc(hwnd: HWND, _l: LPARAM) -> windows::core::BOOL {
        unsafe {
            let mut pid = 0u32;
            let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            // 跳过自己进程的窗口
            if pid == windows::Win32::System::Threading::GetCurrentProcessId() {
                return windows::core::BOOL(1);
            }
            use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
            if IsWindowVisible(hwnd).as_bool() {
                let mut buf = [0u16; 256];
                let n = GetWindowTextW(hwnd, &mut buf);
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
                if n > 0 && ex & WS_EX_TOOLWINDOW.0 == 0 {
                    let title = String::from_utf16_lossy(&buf[..n as usize]);
                    OUT.with(|c| c.borrow_mut().push((title.trim().to_string(), hwnd)));
                }
            }
            windows::core::BOOL(1)
        }
    }
    OUT.with(|c| c.borrow_mut().clear());
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(0));
    }
    OUT.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

#[cfg(windows)]
fn is_own_window(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    if hwnd.0.is_null() {
        return false;
    }
    let mut pid = 0u32;
    unsafe {
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}

#[cfg(windows)]
fn role_name(ct: i32) -> &'static str {
    use windows::Win32::UI::Accessibility::*;
    match ct {
        x if x == UIA_ButtonControlTypeId.0 => "按钮",
        x if x == UIA_MenuItemControlTypeId.0 => "菜单项",
        x if x == UIA_MenuControlTypeId.0 => "菜单",
        x if x == UIA_TabItemControlTypeId.0 => "选项卡",
        x if x == UIA_CheckBoxControlTypeId.0 => "复选框",
        x if x == UIA_RadioButtonControlTypeId.0 => "单选钮",
        x if x == UIA_ComboBoxControlTypeId.0 => "下拉框",
        x if x == UIA_EditControlTypeId.0 => "输入框",
        x if x == UIA_DocumentControlTypeId.0 => "文档",
        x if x == UIA_HyperlinkControlTypeId.0 => "链接",
        x if x == UIA_ListItemControlTypeId.0 => "列表项",
        x if x == UIA_ListControlTypeId.0 => "列表",
        x if x == UIA_TreeControlTypeId.0 => "树",
        x if x == UIA_TreeItemControlTypeId.0 => "树项",
        x if x == UIA_DataItemControlTypeId.0 => "数据项",
        x if x == UIA_SliderControlTypeId.0 => "滑块",
        x if x == UIA_SpinnerControlTypeId.0 => "步进器",
        x if x == UIA_SplitButtonControlTypeId.0 => "分段按钮",
        x if x == UIA_ToolBarControlTypeId.0 => "工具栏",
        x if x == UIA_WindowControlTypeId.0 => "窗口",
        x if x == UIA_PaneControlTypeId.0 => "面板",
        x if x == UIA_GroupControlTypeId.0 => "分组",
        x if x == UIA_TextControlTypeId.0 => "文本",
        x if x == UIA_TableControlTypeId.0 => "表格",
        x if x == UIA_ImageControlTypeId.0 => "图片",
        _ => "元素",
    }
}

fn clean_text(s: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    let out = out.trim();
    if out.chars().count() > max {
        let t: String = out.chars().take(max).collect();
        format!("{t}…")
    } else {
        out.to_string()
    }
}

/* ================= 命令运行（带安全限制） ================= */

const BLOCKED_PATTERNS: &[&str] = &[
    "format-volume", "format.com", "mkfs", "del ", "erase ", "rd ", "rmdir",
    "remove-item", "ri ", "rm -", "rm ", "diskpart", "cipher /w", "shutdown",
    "stop-computer", "restart-computer", "reg delete", "reg add", "vssadmin",
    "bcdedit", "clear-disk", "initialize-disk", "invoke-expression", "iex ",
    "iex(", "|iex", "| iex", "start-process -verb runas",
    // 编码执行类：powershell -EncodedCommand <b64> 的命令体是 Base64，
    // 明文匹配全部失效，必须连编码通道一起拦
    "encodedcommand", " -enc ", " -ec ", "frombase64string", "|enc",
];

fn has_drive_letter(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(1)).any(|i| b[i].is_ascii_alphabetic() && b[i + 1] == b':')
}

/// 危险命令判定（纯函数，供单测回归保护）。
/// 返回命中的模式；课堂环境下删除 / 格式化 / 关机 / 远程代码执行类一律拒绝。
pub fn is_blocked(command: &str) -> Option<&'static str> {
    // .exe 归一化：reg.exe add / shutdown.exe 这类写法此前绕过黑名单
    let lower = command.to_lowercase().replace(".exe", "");
    // format 特判：仅当指向盘符（format C:）才算格式化，
    // 避免 -Format 'yyyy-MM-dd' 这类参数误伤
    if lower.contains("format ") && has_drive_letter(&lower) {
        return Some("format");
    }
    BLOCKED_PATTERNS.iter().find(|pat| lower.contains(*pat)).copied()
}

pub async fn run_command(command: &str) -> ToolResult {
    if let Some(pat) = is_blocked(command) {
        return Err(format!("安全限制：课堂环境下不允许执行「{pat}」类命令（删除/格式化/关机等）"));
    }
    powershell_raw(command)
        .await
        .map(|out| {
            if out.is_empty() {
                "命令执行完成（无输出）".to_string()
            } else {
                let short: String = out.chars().take(4000).collect();
                short
            }
        })
}

/* ================= 打开 / 文件 ================= */

pub async fn open_path(path: &str) -> ToolResult {
    let p = shellexpand(path);
    if p.starts_with("http://") || p.starts_with("https://") {
        return run_start(&p).await;
    }
    let meta = std::fs::metadata(&p).map_err(|_| format!("路径不存在: {p}"))?;
    if meta.is_dir() {
        // 目录用 explorer，保留语义
        let mut cmd = tokio::process::Command::new("explorer");
        cmd.arg(&p);
        #[cfg(windows)]
        {
            cmd.creation_flags(0x08000000);
        }
        let _ = cmd.spawn();
        Ok(format!("已打开文件夹: {p}"))
    } else {
        run_start(&p).await
    }
}

async fn run_start(arg: &str) -> ToolResult {
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.args(["/C", "start", "", arg]);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }
    cmd.spawn().map_err(|e| format!("启动失败: {e}"))?;
    Ok(format!("已打开: {arg}"))
}

pub async fn open_app(name: &str) -> ToolResult {
    run_start(name).await
}

fn shellexpand(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("USERPROFILE") {
            return format!("{home}\\{}", rest.replace('/', "\\"));
        }
    }
    p.to_string()
}

pub fn list_dir(path: &str) -> ToolResult {
    let p = shellexpand(path);
    let entries = std::fs::read_dir(&p).map_err(|e| format!("读取失败: {e}"))?;
    let mut lines = Vec::new();
    let mut seen = 0usize;
    for e in entries {
        seen += 1;
        if seen > 200 {
            break;
        }
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().to_string();
        let meta = e.metadata().map_err(|e| e.to_string())?;
        if meta.is_dir() {
            lines.push(format!("📁 {name}/"));
        } else {
            let size = meta.len();
            let size_s = if size > 1024 * 1024 {
                format!("{:.1} MB", size as f64 / 1048576.0)
            } else {
                format!("{:.0} KB", size as f64 / 1024.0)
            };
            lines.push(format!("📄 {name} ({size_s})"));
        }
    }
    if lines.is_empty() {
        return Ok(format!("{p} 是空目录"));
    }
    if seen > 200 {
        lines.push("…（仅显示前 200 项）".into());
    }
    Ok(lines.join("\n"))
}

fn read_file(v: &Value) -> ToolResult {
    let p = shellexpand(str(v, "path")?);
    let max = v.get("max_chars").and_then(|m| m.as_i64()).unwrap_or(4000) as usize;
    let bytes = std::fs::read(&p).map_err(|e| format!("读取失败: {e}"))?;
    if bytes.iter().take(1000).any(|b| *b == 0) {
        return Err("这是二进制文件，无法以文本方式读取".into());
    }
    let text = String::from_utf8_lossy(&bytes);
    let n = text.chars().count();
    let short: String = text.chars().take(max).collect();
    Ok(if n > max {
        format!("{short}\n…（共 {n} 字符，已截断）")
    } else {
        short
    })
}

fn write_file(v: &Value) -> ToolResult {
    let p = shellexpand(str(v, "path")?);
    let content = str(v, "content")?;
    let append = v.get("append").and_then(|a| a.as_bool()).unwrap_or(false);
    if let Some(parent) = std::path::Path::new(&p).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if append {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
            .map_err(|e| format!("打开失败: {e}"))?;
        f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    } else {
        std::fs::write(&p, content).map_err(|e| e.to_string())?;
    }
    Ok(format!("已写入 {p}（{} 字符，{}）", content.chars().count(), if append { "追加" } else { "覆盖" }))
}

fn search_files(dir: &str, pattern: &str) -> ToolResult {
    let root = shellexpand(dir);
    let pat = pattern.to_lowercase();
    let mut hits = Vec::new();
    let mut budget = 20000u32; // 节点预算：宽目录不至于遍历过久
    walk(&root, &pat, 0, &mut hits, &mut budget);
    if hits.is_empty() {
        Ok(format!("在 {dir} 下未找到匹配「{pattern}」的文件"))
    } else {
        let shown: Vec<String> = hits.iter().take(50).cloned().collect();
        Ok(format!(
            "找到 {} 个（最多显示 50）：\n{}",
            hits.len(),
            shown.join("\n")
        ))
    }
}

fn walk(dir: &str, pattern: &str, depth: u32, hits: &mut Vec<String>, budget: &mut u32) {
    if depth > 6 || hits.len() >= 200 || *budget == 0 {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.contains(pattern) {
                hits.push(path.to_string_lossy().to_string());
            }
            if path.is_dir() {
                walk(path.to_string_lossy().as_ref(), pattern, depth + 1, hits, budget);
            }
        }
    }
}

/* ================= 屏幕信息（给 system prompt） ================= */

#[cfg(windows)]
pub fn screen_size() -> (i32, i32) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
    }
}

#[cfg(not(windows))]
pub fn screen_size() -> (i32, i32) {
    (1920, 1080)
}

/* ================= 自测 ================= */
