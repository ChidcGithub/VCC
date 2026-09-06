# -*- coding: utf-8 -*-
"""egui 迁移 patch 第四轮：windows 0.61 BOOL/COLORREF、rwh 0.6.2、egui Panel、cpal。"""
import sys, os

ROOT = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "src")

def apply(path, patches):
    p = os.path.normpath(os.path.join(ROOT, path))
    with open(p, "r", encoding="utf-8", newline="") as f:
        src = f.read()
    for i, (old, new) in enumerate(patches):
        n = src.count(old)
        if n != 1:
            print(f"[FAIL] {path} patch#{i} 命中 {n} 次（应为 1）")
            print(old[:90].replace("\n", "\\n"))
            sys.exit(1)
        src = src.replace(old, new)
    with open(p, "w", encoding="utf-8", newline="") as f:
        f.write(src)
    print(f"[OK] {path}: {len(patches)} patches")

# ============ overlay.rs ============
OVERLAY = [
    # windows 0.61：BOOL 移到 windows::core
    (
"""    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::*;""",
"""    use windows::core::{w, PCWSTR, BOOL};
    use windows::Win32::Foundation::*;"""),
    # COLOR → COLORREF
    (
"""        let _ = SetLayeredWindowAttributes(hwnd, COLOR(0), 255, LWA_ALPHA);""",
"""        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);"""),
    # DefWindowProcW ABI 显式包装
    (
"""        let wc = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),""",
"""        unsafe extern "system" fn wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),"""),
    # rwh 0.6.2：非 unsafe trait + NonZeroIsize
    (
"""unsafe impl raw_window_handle::HasWindowHandle for HwndHost {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        use raw_window_handle::{RawWindowHandle, Win32WindowHandle, WindowHandle};
        let hwnd = std::ptr::NonNull::new(self.0 as *mut core::ffi::c_void)
            .expect("hwnd 不能为空");
        let w32 = Win32WindowHandle::new(hwnd);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(w32)) })
    }
}""",
"""impl raw_window_handle::HasWindowHandle for HwndHost {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        use raw_window_handle::{RawWindowHandle, Win32WindowHandle, WindowHandle};
        let hwnd = std::num::NonZeroIsize::new(self.0).expect("hwnd 不能为 0");
        let mut w32 = Win32WindowHandle::new(hwnd);
        w32.hinstance = None;
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(w32)) })
    }
}"""),
    # wry WebContext::new 要 Option
    (
"""    let mut webctx =
        wry::WebContext::new(crate::config::data_dir().join("webview2"));""",
"""    let mut webctx = wry::WebContext::new(Some(
        crate::config::data_dir().join("webview2"),
    ));"""),
    # 托盘菜单 accelerator 参数类型
    (
"""    let m_show = tray_icon::menu::MenuItem::with_id("show", "显示主窗口", true, None::<&str>);
    let m_quit = tray_icon::menu::MenuItem::with_id("quit", "退出 VCC", true, None::<&str>);""",
"""    let m_show = tray_icon::menu::MenuItem::with_id("show", "显示主窗口", true, None);
    let m_quit = tray_icon::menu::MenuItem::with_id("quit", "退出 VCC", true, None);"""),
]

# ============ recorder.rs ============
RECORDER = [
    (
"""    let rate = cfg.sample_rate().0;""",
"""    let rate = cfg.sample_rate();"""),
    (
"""    let sink = samples.clone();
    let stream = match cfg.sample_format() {""",
"""    let sink = samples.clone();
    let fmt = cfg.sample_format();
    let stream_cfg: cpal::StreamConfig = cfg.into();
    let stream = match fmt {"""),
    (
"""        cpal::SampleFormat::I16 => device.build_input_stream(
            &cfg.into(),""",
"""        cpal::SampleFormat::I16 => device.build_input_stream(
            stream_cfg.clone(),"""),
    (
"""        cpal::SampleFormat::F32 => device.build_input_stream(
            &cfg.into(),""",
"""        cpal::SampleFormat::F32 => device.build_input_stream(
            stream_cfg.clone(),"""),
]

# ============ ui.rs ============
UI = [
    # TextEdit frame 参数变了，直接去掉
    (
"""                .desired_rows(1)
                .frame(false)
                .show(ui);""",
"""                .desired_rows(1)
                .show(ui);"""),
    # egui 0.36 面板统一为 Panel
    (
"""        egui::SidePanel::left("sidebar")
            .exact_width(261.0)""",
"""        egui::Panel::left("sidebar")
            .exact_size(261.0)"""),
    (
"""        egui::TopBottomPanel::bottom("input")""",
"""        egui::Panel::bottom("input")"""),
    # radio_value 替代 RadioButton
    (
"""                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("语音模型档位").color(th.text));
                    if egui::RadioButton::new(&mut self.cfg.voice_model, "fast")
                        .text("fast（默认，更快）")
                        .ui(ui)
                        .clicked()
                    {
                        self.cfg.voice_model = "fast".into();
                    }
                    if egui::RadioButton::new(&mut self.cfg.voice_model, "quality")
                        .text("quality（更准）")
                        .ui(ui)
                        .clicked()
                    {
                        self.cfg.voice_model = "quality".into();
                    }
                });""",
"""                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("语音模型档位").color(th.text));
                    ui.radio_value(&mut self.cfg.voice_model, "fast".to_string(), "fast（默认，更快）");
                    ui.radio_value(&mut self.cfg.voice_model, "quality".to_string(), "quality（更准）");
                });"""),
    (
"""                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("识别语言").color(th.text));
                    for lang in ["zh", "en", "auto"] {
                        if egui::RadioButton::new(&mut self.cfg.voice_lang, lang)
                            .text(lang)
                            .ui(ui)
                            .clicked()
                        {
                            self.cfg.voice_lang = lang.into();
                        }
                    }
                });""",
"""                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("识别语言").color(th.text));
                    ui.radio_value(&mut self.cfg.voice_lang, "zh".to_string(), "中文");
                    ui.radio_value(&mut self.cfg.voice_lang, "en".to_string(), "English");
                    ui.radio_value(&mut self.cfg.voice_lang, "auto".to_string(), "自动");
                });"""),
]

apply("overlay.rs", OVERLAY)
apply("recorder.rs", RECORDER)
apply("ui.rs", UI)
print("ALL PATCHES APPLIED")
