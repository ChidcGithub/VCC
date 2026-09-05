# -*- coding: utf-8 -*-
# show_dialog: WebView 弹窗 → Win32 原生 TaskDialog（comctl32 v6 动态加载）
import io, json

ROOT = r"D:\My things\Learn\高二\VCC"
ok = []

# ---------- 1) Cargo.toml: 加 features ----------
p = ROOT + r"\src-tauri\Cargo.toml"
s = io.open(p, encoding="utf-8").read()
assert '"Win32_UI_Controls"' not in s
s = s.replace('  "Win32_System_Com",',
              '  "Win32_System_Com",\n  "Win32_System_LibraryLoader",')
s = s.replace('  "Win32_UI_Accessibility",',
              '  "Win32_UI_Accessibility",\n  "Win32_UI_Controls",')
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
ok.append("Cargo.toml features")

# ---------- 2) tools.rs: 重写弹窗段 ----------
p = ROOT + r"\src-tauri\src\tools.rs"
s = io.open(p, encoding="utf-8").read()
START = "/* ================= AI 自定义弹窗"
END = "/* ================= 剪贴板 ================= */"
i, j = s.index(START), s.index(END)

NEW = r'''/* ================= AI 自定义弹窗（Win32 原生 TaskDialog，回传用户选择） ================= */

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
        TD_ERROR_ICON, TD_INFORMATION_ICON, TD_WARNING_ICON, TDF_ALLOW_DIALOG_CANCELLATION,
        TDF_CALLBACK_TIMER, TDF_POSITION_RELATIVE_TO_WINDOW, TDN_TIMER,
    };
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, IDCANCEL};

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
            let _ = SendMessageW(hwnd, TDM_CLOSE, WPARAM(0), LPARAM(0));
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
        0 => format!("弹窗 {} 秒内未得到响应，已自动关闭", p.timeout_secs),
        _ => "弹窗已关闭（未选择）".into(),
    })
}

'''
s = s[:i] + NEW + s[j:]
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
ok.append("tools.rs dialog section")

# ---------- 3) lib.rs: 拆事件桥 + dialog_ready ----------
p = ROOT + r"\src-tauri\src\lib.rs"
s = io.open(p, encoding="utf-8").read()

bridge = '''    // 弹窗结果桥：dialog.js emit("vcc://dialog-result") → 转发给当前等待中的 show_dialog 工具
    app.listen("vcc://dialog-result", |ev| {
        let payload = ev.payload().to_string();
        if let Some(tx) = tools::DIALOG_RESULT_TX.lock().unwrap().as_ref() {
            let _ = tx.send(payload);
        }
    });

'''
assert bridge in s
s = s.replace(bridge, "")
assert "DIALOG_RESULT_TX" not in s

cmd = '''/// 弹窗页面就绪：补发本次弹窗参数（防 emit 先于 JS listen 的竞态）
#[tauri::command]
fn dialog_ready(app: AppHandle) {
    use tauri::Emitter;
    if let Some(p) = tools::DIALOG_PAYLOAD.lock().unwrap().as_ref() {
        let _ = app.emit_to("dialog", "vcc://dialog-set", p.clone());
    }
}
'''
assert cmd in s
s = s.replace(cmd, "")
assert "DIALOG_PAYLOAD" not in s

old_h = "            voice::probe_env,\n            dialog_ready\n        ])"
assert old_h in s
s = s.replace(old_h, "            voice::probe_env\n        ])")
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
ok.append("lib.rs bridge+command removed")

# ---------- 4) capability: 去掉 dialog 窗口 ----------
p = ROOT + r"\src-tauri\capabilities\default.json"
d = json.load(io.open(p, encoding="utf-8"))
d["windows"] = [w for w in d["windows"] if w != "dialog"]
io.open(p, "w", encoding="utf-8", newline="\n").write(json.dumps(d, indent=2, ensure_ascii=False) + "\n")
ok.append("capability windows=" + repr(d["windows"]))

# ---------- 5) main.rs: 测试改 dialog_params ----------
p = ROOT + r"\src-tauri\src\main.rs"
s = io.open(p, encoding="utf-8").read()
old_t = '''    /// 弹窗参数：缺省按钮补「好/primary」，超时 clamp
    #[test]
    fn dialog_payload_defaults() {
        let v: serde_json::Value = serde_json::json!({"body": "要继续吗？"});
        let (payload, timeout) = dialog_payload(&v).expect("payload");
        assert_eq!(timeout, 120);
        let p: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(p["title"], "提示");
        assert_eq!(p["buttons"][0]["label"], "好");
        assert_eq!(p["buttons"][0]["style"], "primary");

        let v2: serde_json::Value = serde_json::json!({
            "title": "确认", "body": "删除这个吗？", "timeout_secs": 5,
            "buttons": [{"label": "删除", "style": "danger"}, {"label": "取消", "style": "primary"}]
        });
        let (payload2, timeout2) = dialog_payload(&v2).unwrap();
        assert_eq!(timeout2, 10, "超时应 clamp 到 10s 下限");
        let p2: serde_json::Value = serde_json::from_str(&payload2).unwrap();
        assert_eq!(p2["buttons"][0]["style"], "danger");
        assert_eq!(p2["buttons"][1]["label"], "取消");
    }

    /// 弹窗正文为空必须报错（防止空弹窗骚扰用户）
    #[test]
    fn dialog_payload_requires_body() {
        let v: serde_json::Value = serde_json::json!({"title": "hi"});
        assert!(dialog_payload(&v).is_err());
    }'''
assert old_t in s
new_t = '''    /// 弹窗参数：缺省按钮补「好」，默认焦点=第一个 primary，danger 触发警告图标，超时 clamp
    #[test]
    fn dialog_params_defaults() {
        let v: serde_json::Value = serde_json::json!({"body": "要继续吗？"});
        let p = dialog_params(&v).expect("params");
        assert_eq!(p.timeout_secs, 120);
        assert_eq!(p.buttons, vec!["好"]);
        assert_eq!(p.default_idx, 0);
        assert!(p.info_icon && !p.warn_icon);

        let v2: serde_json::Value = serde_json::json!({
            "title": "确认", "body": "删除这个吗？", "timeout_secs": 5,
            "buttons": [{"label": "删除", "style": "danger"}, {"label": "取消", "style": "primary"}]
        });
        let p2 = dialog_params(&v2).unwrap();
        assert_eq!(p2.timeout_secs, 10, "超时应 clamp 到 10s 下限");
        assert!(p2.warn_icon && !p2.info_icon, "danger 优先用警告图标");
        assert_eq!(p2.default_idx, 1, "默认焦点=第一个 primary（取消）");
        assert_eq!(p2.buttons, vec!["删除", "取消"]);
    }

    /// 弹窗正文为空必须报错（防止空弹窗骚扰用户）
    #[test]
    fn dialog_params_requires_body() {
        let v: serde_json::Value = serde_json::json!({"title": "hi"});
        assert!(dialog_params(&v).is_err());
    }'''
s = s.replace(old_t, new_t)
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
ok.append("main.rs dialog tests")

for o in ok:
    print("OK:", o)
