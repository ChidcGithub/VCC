# -*- coding: utf-8 -*-
"""egui 迁移 patch 第三轮：App trait 拆分 / cpal / windows 路径 / 单实例双 bind。"""
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
    # HINSTANCE 正确路径（Foundation）
    (
"""        let hmodule = GetModuleHandleW(None).expect("GetModuleHandleW");
        let hinstance = windows::Win32::UI::WindowsAndMessaging::HINSTANCE(hmodule.0);""",
"""        let hmodule = GetModuleHandleW(None).expect("GetModuleHandleW");
        let hinstance = HINSTANCE(hmodule.0);"""),
    # MONITORENUMPROC 不是元组构造
    (
"""            Some(MONITORENUMPROC(Some(cb))),""",
"""            Some(cb),"""),
    # SetFocus 在 KeyboardAndFocus 模块，去掉（SetForegroundWindow 足够）
    (
"""            let _ = SetForegroundWindow(HWND(hwnd as _));
            let _ = SetFocus(HWND(hwnd as _));""",
"""            let _ = SetForegroundWindow(HWND(hwnd as _));"""),
    # GlobalHotKeyManager 顶层导入（setup_tray_hotkey 签名用）
    (
"""use wry::WebView;""",
"""use global_hotkey::GlobalHotKeyManager;
use wry::WebView;"""),
]

# ============ recorder.rs ============
RECORDER = [
    # cpal 0.18：free fn 已移除，走 default_host()
    (
"""    let device = cpal::default_input_device().ok_or("没有找到麦克风设备")?;""",
"""    let device = cpal::default_host()
        .default_input_device()
        .ok_or("没有找到麦克风设备")?;"""),
    # hound 无 into_inner：手写 WAV 头（PCM 16bit）
    (
"""    /// 停止录音并编码 WAV base64（消耗 self：Stream drop 停流）
    pub fn stop(self) -> Result<String, String> {
        use hound::{SampleFormat, WavSpec, WavWriter};

        let samples = match self.samples.lock() {
            Ok(s) => s.clone(),
            Err(_) => Vec::new(),
        };
        if samples.len() < self.sample_rate as usize / 5 {
            return Err("录音太短".into());
        }

        let spec = WavSpec {
            channels: self.channels,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = WavWriter::new(cursor, spec).map_err(|e| format!("WAV 创建失败: {e}"))?;
        for x in &samples {
            writer.write_sample(*x).map_err(|e| format!("WAV 写入失败: {e}"))?;
        }
        let bytes = writer
            .into_inner()
            .map_err(|e| format!("WAV 收尾失败: {e}"))?
            .into_inner();
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }""",
"""    /// 停止录音并编码 WAV base64（消耗 self：Stream drop 停流）。
    /// WAV 头手写（PCM 16bit）——hound 3.5 的 WavWriter 没有 into_inner，取不出字节。
    pub fn stop(self) -> Result<String, String> {
        let samples = match self.samples.lock() {
            Ok(s) => s.clone(),
            Err(_) => Vec::new(),
        };
        if samples.len() < self.sample_rate as usize / 5 {
            return Err("录音太短".into());
        }
        let bytes = wav_bytes(&samples, self.sample_rate, self.channels);
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }
}

/// 标准 PCM WAV（44 字节头 + LE i16 数据）
fn wav_bytes(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes()); // fmt 块长
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes()); // byte rate
    b.extend_from_slice(&(channels * 2).to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b"""),
]

# ============ lib.rs ============
LIB = [
    # 单实例双 bind 修复：bind 一次，listener 移交线程
    (
"""    // 单实例：端口被占 → 通知已有实例呼出主窗后退出
    if std::net::TcpListener::bind(("127.0.0.1", SINGLE_INSTANCE_PORT)).is_err() {
        if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", SINGLE_INSTANCE_PORT)) {
            let _ = std::io::Write::write_all(&mut s, b"show\\n");
        }
        return;
    }""",
"""    // 单实例：端口被占 → 通知已有实例呼出主窗后退出
    let instance_listener =
        match std::net::TcpListener::bind(("127.0.0.1", SINGLE_INSTANCE_PORT)) {
            Ok(l) => l,
            Err(_) => {
                if let Ok(mut s) =
                    std::net::TcpStream::connect(("127.0.0.1", SINGLE_INSTANCE_PORT))
                {
                    let _ = std::io::Write::write_all(&mut s, b"show\\n");
                }
                return;
            }
        };"""),
    (
"""    // 二次启动转发线程（单实例监听）
    {
        let ctl2 = ctl.clone();
        std::thread::spawn(move || {
            if let Ok(listener) =
                std::net::TcpListener::bind(("127.0.0.1", SINGLE_INSTANCE_PORT))
            {
                for stream in listener.incoming() {
                    let mut s = match stream {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let mut buf = [0u8; 16];
                    if std::io::Read::read(&mut s, &mut buf).is_ok() {
                        ctl2.request_show(); // 泵线程轮询执行 Win32 呼出
                    }
                }
            }
        });
    }""",
"""    // 二次启动转发线程（单实例监听）
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
    }"""),
]

# ============ ui.rs（App trait 拆分） ============
UI = [
    (
"""impl eframe::App for VccApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.hwnd_done {
            self.detect_hwnd();
        }
        self.drain_events();

        // done → 900ms 后回落 idle
        if let Some(t) = self.done_at {
            if t.elapsed() >= std::time::Duration::from_millis(900) && self.phase == "done" {
                self.done_at = None;
                self.set_phase("idle");
            }
        }

        // 录音：电平广播 + 60s 上限
        if let Some(rec) = &self.recorder {
            let l = rec.level();
            let _ = self.pump.send(PumpMsg::Level(l));
            if rec.elapsed() >= std::time::Duration::from_secs(60) {
                ctx.request_repaint();
                self.toggle_mic();
            }
        }

        // ESC 快速收起（课堂场景一键隐藏；agent 后台继续跑）
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            #[cfg(windows)]
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
                if self.hwnd != 0 {
                    let _ = ShowWindow(windows::Win32::Foundation::HWND(self.hwnd as _), SW_HIDE);
                }
            }
            self.ctl.main_visible.store(false, Ordering::SeqCst);
        }

        let th = theme(self.dark);""",
"""impl eframe::App for VccApp {
    /// 非 UI 杂务：事件、计时、录音电平——窗口隐藏时也会被调用
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.hwnd_done {
            self.detect_hwnd();
        }
        self.drain_events();

        // done → 900ms 后回落 idle
        if let Some(t) = self.done_at {
            if t.elapsed() >= std::time::Duration::from_millis(900) && self.phase == "done" {
                self.done_at = None;
                self.set_phase("idle");
            }
        }

        // 录音：电平广播 + 60s 上限
        if let Some(rec) = &self.recorder {
            let l = rec.level();
            let _ = self.pump.send(PumpMsg::Level(l));
            if rec.elapsed() >= std::time::Duration::from_secs(60) {
                ctx.request_repaint();
                self.toggle_mic();
            }
        }

        // ESC 快速收起（课堂场景一键隐藏；agent 后台继续跑）
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            #[cfg(windows)]
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
                if self.hwnd != 0 {
                    let _ = ShowWindow(windows::Win32::Foundation::HWND(self.hwnd as _), SW_HIDE);
                }
            }
            self.ctl.main_visible.store(false, Ordering::SeqCst);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let th = theme(self.dark);"""),
    (
"""        if self.show_settings {
            self.settings_ui(ctx);
        }

        // 流式/录音期间持续重绘
        if self.stream_text.is_some() || self.recorder.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn on_close_event(&mut self) -> bool {
        // 点关闭 = 退出整个应用（托盘常驻语义维持：真退出走托盘「退出 VCC」也可以）
        self.ctl.exit.store(true, Ordering::SeqCst);
        true
    }
}""",
"""        if self.show_settings {
            self.settings_ui(&ctx);
        }

        // 流式/录音期间持续重绘
        if self.stream_text.is_some() || self.recorder.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}"""),
    (
""".show(ctx, |ui| self.sidebar(ui))""",
""".show(&ctx, |ui| self.sidebar(ui))"""),
    (
""".show(ctx, |ui| self.input_bar(ui))""",
""".show(&ctx, |ui| self.input_bar(ui))"""),
    (
""".show(ctx, |ui| self.message_view(ui))""",
""".show(&ctx, |ui| self.message_view(ui))"""),
]

apply("overlay.rs", OVERLAY)
apply("recorder.rs", RECORDER)
apply("lib.rs", LIB)
apply("ui.rs", UI)
print("ALL PATCHES APPLIED")
