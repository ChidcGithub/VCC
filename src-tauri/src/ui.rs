/* ---------- eframe 对话主窗口（DeepSeek 式布局） ---------- */
/* 侧栏 261px（会话分组列表 + 新对话）+ 消息流（气泡/工具行/流式/markdown）+
   底部胶囊输入条（Enter 发送 / Shift+Enter 换行 / 麦克风）+ 设置浮层。
   主题：深浅双套色板（对标 chat.deepseek.com 观感），品牌蓝 #4D6BFE。 */

use crate::bus::{EventTx, PumpMsg, SharedWinCtl, UiEvent};
use crate::recorder::Recorder;
use crate::SharedState;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};

/* ---------- 主题 ---------- */

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: egui::Color32,
    pub sidebar: egui::Color32,
    pub bubble_user: egui::Color32,
    pub row_hover: egui::Color32,
    pub text: egui::Color32,
    pub weak: egui::Color32,
    pub accent: egui::Color32,
    pub border: egui::Color32,
    pub input_bg: egui::Color32,
    pub ok: egui::Color32,
    pub bad: egui::Color32,
}

pub fn theme(dark: bool) -> Theme {
    if dark {
        Theme {
            bg: egui::Color32::from_rgb(0x29, 0x2A, 0x2D),
            sidebar: egui::Color32::from_rgb(0xED, 0xEE, 0xEF), // 占位，dark 下不会用
            bubble_user: egui::Color32::from_rgb(0x3B, 0x43, 0x58),
            row_hover: egui::Color32::from_rgb(0x35, 0x36, 0x3A),
            text: egui::Color32::from_rgb(0xE8, 0xE8, 0xE9),
            weak: egui::Color32::from_rgb(0x8A, 0x8F, 0x98),
            accent: egui::Color32::from_rgb(0x4D, 0x6B, 0xFE),
            border: egui::Color32::from_rgb(0x3A, 0x3B, 0x3F),
            input_bg: egui::Color32::from_rgb(0x32, 0x33, 0x37),
            ok: egui::Color32::from_rgb(0x4C, 0xC3, 0x87),
            bad: egui::Color32::from_rgb(0xE5, 0x5B, 0x4D),
        }
    } else {
        Theme {
            bg: egui::Color32::WHITE,
            sidebar: egui::Color32::WHITE,
            bubble_user: egui::Color32::from_rgb(0xE8, 0xEC, 0xF9),
            row_hover: egui::Color32::from_rgb(0xF0, 0xF1, 0xF3),
            text: egui::Color32::from_rgb(0x1B, 0x1B, 0x1F),
            weak: egui::Color32::from_rgb(0x7A, 0x7F, 0x87),
            accent: egui::Color32::from_rgb(0x4D, 0x6B, 0xFE),
            border: egui::Color32::from_rgb(0xE2, 0xE4, 0xE9),
            input_bg: egui::Color32::from_rgb(0xF4, 0xF5, 0xF7),
            ok: egui::Color32::from_rgb(0x1F, 0x9D, 0x55),
            bad: egui::Color32::from_rgb(0xD9, 0x30, 0x26),
        }
    }
}

impl Theme {
    fn md(&self) -> crate::md::MdStyle {
        crate::md::MdStyle {
            text: self.text,
            weak: self.weak,
            code_bg: self.input_bg,
            accent: self.accent,
        }
    }
}

/* ---------- 展示消息 ---------- */

enum UiMsg {
    User(String),
    Ai(String),
    Err(String),
    Tool { id: String, label: String, done: Option<bool> },
}

pub struct VccApp {
    state: SharedState,
    ev: EventTx,
    ev_rx: Receiver<UiEvent>,
    pump: Sender<PumpMsg>,
    ctl: SharedWinCtl,
    cfg: crate::config::Config,

    dark: bool,
    msgs: Vec<UiMsg>,
    stream_text: Option<String>,
    phase: String,
    done_at: Option<std::time::Instant>,
    sessions: Vec<crate::memory::SessionMeta>,
    current_sid: String,
    input: String,
    show_settings: bool,
    renaming: Option<String>,
    rename_buf: String,
    recorder: Option<Recorder>,
    hwnd_done: bool,
    hwnd: isize,
    always_on_top: bool,
}

impl VccApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        state: SharedState,
        ev: EventTx,
        ev_rx: Receiver<UiEvent>,
        pump: Sender<PumpMsg>,
        ctl: SharedWinCtl,
    ) -> Self {
        let cfg = crate::config::load();
        let dark = cfg.theme != "light";
        install_fonts(&cc.egui_ctx);
        apply_visuals(&cc.egui_ctx, dark);

        // 启动恢复：渲染当前会话最近 40 条（tool 中间轮过滤）
        let (sid, _) = crate::memory::load_history();
        let msgs = msgs_from_history(&state);
        let sessions = crate::memory::list_sessions();

        let always_on_top = cfg.always_on_top;
        Self {
            state,
            ev,
            ev_rx,
            pump,
            ctl,
            cfg,
            dark,
            msgs,
            stream_text: None,
            phase: "idle".into(),
            done_at: None,
            sessions,
            current_sid: sid,
            input: String::new(),
            show_settings: false,
            renaming: None,
            rename_buf: String::new(),
            recorder: None,
            hwnd_done: false,
            hwnd: 0,
            always_on_top,
        }
    }

    /* ---------- 事件处理 ---------- */

    fn drain_events(&mut self) {
        while let Ok(e) = self.ev_rx.try_recv() {
            match e {
                UiEvent::ChatStart => {
                    self.stream_text = Some(String::new());
                }
                UiEvent::ChatDelta(t) => {
                    if let Some(s) = &mut self.stream_text {
                        s.push_str(&t);
                    }
                }
                UiEvent::ChatEnd => {
                    if let Some(s) = self.stream_text.take() {
                        if !s.trim().is_empty() {
                            self.msgs.push(UiMsg::Ai(s));
                        }
                    }
                }
                UiEvent::ChatBubble { role, text } => {
                    if role == "err" {
                        self.msgs.push(UiMsg::Err(text));
                    } else {
                        self.msgs.push(UiMsg::Ai(text));
                    }
                }
                UiEvent::ToolStart { id, label } => {
                    self.msgs.push(UiMsg::Tool { id, label, done: None });
                }
                UiEvent::ToolDone { id, ok } => {
                    if let Some(m) = self.msgs.iter_mut().rev().find(|m| match m {
                        UiMsg::Tool { id: i, done, .. } => i == &id && done.is_none(),
                        _ => false,
                    }) {
                        if let UiMsg::Tool { done, .. } = m {
                            *done = Some(ok);
                        }
                    }
                }
                UiEvent::Phase(p) => {
                    if p == "done" {
                        self.done_at = Some(std::time::Instant::now());
                    } else if p == "idle" {
                        self.done_at = None;
                    }
                    self.phase = p.clone();
                    let _ = self.pump.send(PumpMsg::Phase(p));
                }
                UiEvent::Float { mode, steps, text } => {
                    let _ = self.pump.send(PumpMsg::Float { mode, steps, text });
                }
                UiEvent::Invoked => {
                    self.phase = "summoned".into();
                    self.done_at = None;
                    let _ = self.pump.send(PumpMsg::Phase("summoned".into()));
                }
                UiEvent::Transcribed(r) => self.on_transcribed(r),
            }
        }
    }

    fn on_transcribed(&mut self, r: Result<String, String>) {
        match r {
            Ok(text) => {
                let t = text.trim().to_string();
                if t.is_empty() {
                    self.msgs.push(UiMsg::Err("没听清，再试一次？".into()));
                    self.set_phase("idle");
                } else {
                    self.send(t);
                }
            }
            Err(e) => {
                self.msgs.push(UiMsg::Err(e));
                self.set_phase("idle");
            }
        }
    }

    fn set_phase(&mut self, p: &str) {
        self.phase = p.into();
        let _ = self.pump.send(PumpMsg::Phase(p.into()));
    }

    /* ---------- 动作 ---------- */

    fn send(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if self
            .state
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.msgs
                .push(UiMsg::Err("上一条指令还在执行中，请稍候".into()));
            return;
        }
        self.msgs.push(UiMsg::User(text.clone()));
        let st = self.state.clone();
        let ev = self.ev.clone();
        crate::rt().spawn(async move {
            let r = crate::llm::run_agent(&st, &ev, text).await;
            st.busy.store(false, Ordering::SeqCst);
            if let Err(e) = r {
                ev.send(UiEvent::ChatBubble { role: "err", text: e });
                ev.send(UiEvent::ChatEnd);
                ev.send(UiEvent::Phase("idle".into()));
                ev.send(UiEvent::Float {
                    mode: "done".into(),
                    steps: vec![],
                    text: "出错了，详情见主窗口".into(),
                });
            }
        });
    }

    fn refresh_sessions(&mut self) {
        self.sessions = crate::memory::list_sessions();
        self.current_sid = self
            .state
            .current
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default();
    }

    fn reload_display(&mut self) {
        self.msgs = msgs_from_history(&self.state);
        self.stream_text = None;
    }

    fn new_chat(&mut self) {
        if !acquire_busy(&self.state) {
            self.msgs
                .push(UiMsg::Err("上一条指令还在执行中".into()));
            return;
        }
        let hist = self
            .state
            .history
            .lock()
            .map(|h| h.clone())
            .unwrap_or_default();
        if hist.iter().any(|m| m.role == "user") {
            crate::rt().spawn(crate::memory::summarize_into_memory(hist.clone()));
        }
        if let Ok(mut h) = self.state.history.lock() {
            h.clear();
        }
        let id = crate::memory::new_session();
        if let Ok(mut c) = self.state.current.lock() {
            *c = id.clone();
        }
        self.current_sid = id;
        self.msgs.clear();
        self.stream_text = None;
        self.refresh_sessions();
    }

    fn switch_session(&mut self, id: &str) {
        if id == self.current_sid {
            return;
        }
        if !acquire_busy(&self.state) {
            self.msgs
                .push(UiMsg::Err("上一条指令还在执行中，请稍候再切换".into()));
            return;
        }
        // 写回当前会话
        let cur = self
            .state
            .current
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default();
        if !cur.is_empty() {
            let hist = self
                .state
                .history
                .lock()
                .map(|h| h.clone())
                .unwrap_or_default();
            crate::memory::save_history(&hist);
        }
        match crate::memory::switch_session(id) {
            Ok(msgs) => {
                if let Ok(mut h) = self.state.history.lock() {
                    *h = msgs;
                }
                if let Ok(mut c) = self.state.current.lock() {
                    *c = id.to_string();
                }
                self.current_sid = id.to_string();
                self.reload_display();
            }
            Err(e) => self.msgs.push(UiMsg::Err(e)),
        }
        self.refresh_sessions();
    }

    fn delete_session(&mut self, id: &str) {
        if !acquire_busy(&self.state) {
            self.msgs
                .push(UiMsg::Err("上一条指令还在执行中，请稍候再删除".into()));
            return;
        }
        let cur = self
            .state
            .current
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default();
        if cur == id {
            if let Ok(mut h) = self.state.history.lock() {
                h.clear();
            }
        }
        let new_cur = crate::memory::delete_session(id);
        let final_id = if new_cur.is_empty() {
            crate::memory::new_session()
        } else {
            new_cur
        };
        if cur == id {
            let msgs = crate::memory::switch_session(&final_id).unwrap_or_default();
            if let Ok(mut h) = self.state.history.lock() {
                *h = msgs;
            }
            if let Ok(mut c) = self.state.current.lock() {
                *c = final_id.clone();
            }
            self.current_sid = final_id;
            self.reload_display();
        }
        self.refresh_sessions();
    }

    fn commit_rename(&mut self) {
        if let Some(id) = self.renaming.take() {
            let t = self.rename_buf.trim().to_string();
            if !t.is_empty() {
                crate::memory::rename_session(&id, &t);
            }
        }
        self.rename_buf.clear();
        self.refresh_sessions();
    }

    /* ---------- 语音 ---------- */

    fn toggle_mic(&mut self) {
        if self.recorder.is_some() {
            // 停止 → 转写
            let rec = self.recorder.take().unwrap();
            match rec.stop() {
                Ok(b64) => {
                    self.set_phase("thinking");
                    let ev = self.ev.clone();
                    crate::rt().spawn(async move {
                        let r = crate::voice::transcribe(&b64).await;
                        ev.send(UiEvent::Transcribed(r));
                    });
                }
                Err(e) => {
                    self.msgs.push(UiMsg::Err(e));
                    self.set_phase("idle");
                }
            }
        } else {
            match crate::recorder::start() {
                Ok(rec) => {
                    self.recorder = Some(rec);
                    self.set_phase("listening");
                }
                Err(e) => self.msgs.push(UiMsg::Err(e)),
            }
        }
    }

    /* ---------- 面板 ---------- */

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let th = theme(self.dark);
        // 新对话
        ui.add_space(8.0);
        let new_btn = egui::Button::new(egui::RichText::new("＋ 新对话").color(th.text))
            .fill(th.input_bg)
            .corner_radius(100.0)
            .min_size(egui::vec2(ui.available_width() - 16.0, 32.0));
        if ui
            .horizontal(|ui| {
                ui.add_space(8.0);
                ui.add(new_btn).clicked()
            })
            .inner
        {
            self.new_chat();
        }
        ui.add_space(10.0);

        // 分组：今天/昨天/7 天/更早（按 updated_at 里的「X年X月X日」前缀分组）
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut last_group = String::new();
            let sessions = self.sessions.clone();
            for s in &sessions {
                let g = session_group(&s.updated_at);
                if g != last_group {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(&g).size(11.5).color(th.weak));
                    });
                    ui.add_space(2.0);
                    last_group = g;
                }
                let is_cur = s.id == self.current_sid;
                if self.renaming.as_deref() == Some(s.id.as_str()) {
                    // 重命名态
                    let resp = ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        let r = egui::TextEdit::singleline(&mut self.rename_buf)
                            .desired_width(ui.available_width() - 24.0)
                            .font(egui::TextStyle::Body)
                            .show(ui);
                        r.response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    });
                    if resp.inner {
                        self.commit_rename();
                    }
                    continue;
                }
                let label = egui::RichText::new(&s.title).size(13.0).color(th.text);
                let btn = egui::Button::new(label)
                    .fill(if is_cur { th.row_hover } else { egui::Color32::TRANSPARENT })
                    .corner_radius(12.0)
                    .min_size(egui::vec2(ui.available_width() - 12.0, 40.0));
                let resp = ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.add(btn)
                })
                .inner;
                if resp.clicked() {
                    self.switch_session(&s.id);
                }
                resp.context_menu(|ui| {
                    if ui.button("重命名").clicked() {
                        self.renaming = Some(s.id.clone());
                        self.rename_buf = s.title.clone();
                        ui.close();
                    }
                    if ui.button("删除").clicked() {
                        self.delete_session(&s.id);
                        ui.close();
                    }
                });
            }
        });

        // 底部：设置 + 主题
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                if ui.button(egui::RichText::new("⚙ 设置").color(th.weak)).clicked() {
                    self.show_settings = !self.show_settings;
                }
                let theme_label = if self.dark { "☀ 浅色" } else { "🌙 深色" };
                if ui.button(egui::RichText::new(theme_label).color(th.weak)).clicked() {
                    self.dark = !self.dark;
                    self.cfg.theme = if self.dark { "dark".into() } else { "light".into() };
                    let _ = crate::config::save(&self.cfg);
                    apply_visuals(ui.ctx(), self.dark);
                }
            });
            ui.add_space(6.0);
        });
        ui.add_space(6.0);
    }

    fn message_view(&mut self, ui: &mut egui::Ui) {
        let th = theme(self.dark);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(self.stream_text.is_some())
            .show(ui, |ui| {
                let width = ui.available_width();
                for m in &self.msgs {
                    match m {
                        UiMsg::User(t) => {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                ui.add_space(40.0);
                                egui::Frame::default()
                                    .fill(th.bubble_user)
                                    .corner_radius(12.0)
                                    .inner_margin(egui::Margin::symmetric(12, 8))
                                    .show(ui, |ui| {
                                        ui.set_max_width(width * 0.7);
                                        ui.label(egui::RichText::new(t).color(th.text).size(14.0));
                                    });
                            });
                            ui.add_space(10.0);
                        }
                        UiMsg::Ai(t) => {
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                                ui.add_space(8.0);
                                ui.vertical(|ui| {
                                    ui.set_max_width(width * 0.86);
                                    crate::md::show(ui, t, &th.md());
                                });
                            });
                            ui.add_space(10.0);
                        }
                        UiMsg::Err(t) => {
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(format!("⚠ {t}")).color(th.bad).size(13.5));
                            });
                            ui.add_space(8.0);
                        }
                        UiMsg::Tool { label, done, .. } => {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                match done {
                                    None => {
                                        ui.add(egui::Spinner::new().size(12.0).color(th.accent));
                                        ui.label(egui::RichText::new(label).color(th.weak).size(12.5));
                                    }
                                    Some(true) => {
                                        ui.label(egui::RichText::new("✓").color(th.ok).size(12.5));
                                        ui.label(egui::RichText::new(label).color(th.weak).size(12.5));
                                    }
                                    Some(false) => {
                                        ui.label(egui::RichText::new("✗").color(th.bad).size(12.5));
                                        ui.label(egui::RichText::new(label).color(th.bad).size(12.5));
                                    }
                                }
                            });
                            ui.add_space(6.0);
                        }
                    }
                }
                // 流式气泡
                if let Some(s) = &self.stream_text {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(width * 0.86);
                            if s.is_empty() {
                                ui.label(egui::RichText::new("…").color(th.weak));
                            } else {
                                crate::md::show(ui, s, &th.md());
                                ui.label(egui::RichText::new("▌").color(th.accent).size(13.0));
                            }
                        });
                    });
                }
            });
    }

    fn input_bar(&mut self, ui: &mut egui::Ui) {
        let th = theme(self.dark);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            // 麦克风
            let rec = self.recorder.is_some();
            let mic_text = if rec { "■ 停止" } else { "🎤 语音" };
            let mic_btn = egui::Button::new(
                egui::RichText::new(mic_text).color(if rec { th.bad } else { th.text }),
            )
            .fill(if rec { th.row_hover } else { th.input_bg })
            .corner_radius(100.0)
            .min_size(egui::vec2(74.0, 30.0));
            if ui.add(mic_btn).clicked() {
                self.toggle_mic();
            }
            // 快捷指令（空态时显示）
            let cmds: Vec<String> = if self.cfg.custom_cmds.is_empty() {
                vec!["打开课件文件夹".into(), "把音量调到 30%".into(), "现在几点".into()]
            } else {
                self.cfg.custom_cmds.clone()
            };
            let idle = self.phase == "idle" || self.phase == "summoned" || self.phase == "done";
            if self.msgs.is_empty() && self.stream_text.is_none() && idle {
                for c in cmds.iter().take(4) {
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new(c).size(12.5).color(th.weak))
                                .fill(egui::Color32::TRANSPARENT)
                                .corner_radius(100.0)
                                .stroke(egui::Stroke::new(1.0, th.border)),
                        )
                        .clicked()
                    {
                        self.input = c.clone();
                    }
                }
            }
            // 输入框
            let send_now = ui.input(|i| {
                i.key_pressed(egui::Key::Enter) && !i.modifiers.shift
            });
            let resp = egui::TextEdit::multiline(&mut self.input)
                .hint_text(egui::RichText::new("给 VCC 发送指令…").color(th.weak))
                .desired_width(ui.available_width() - 90.0)
                .desired_rows(1)
                .show(ui);
            resp.response.surrender_focus();
            if send_now && !self.input.trim().is_empty() {
                let text = std::mem::take(&mut self.input);
                self.send(text);
            }
            // 发送按钮
            let can_send = !self.input.trim().is_empty();
            let send_btn = egui::Button::new(
                egui::RichText::new("发送").color(if can_send { egui::Color32::WHITE } else { th.weak }),
            )
            .fill(if can_send { th.accent } else { th.input_bg })
            .corner_radius(100.0)
            .min_size(egui::vec2(64.0, 30.0));
            if ui.add(send_btn).clicked() && can_send {
                let text = std::mem::take(&mut self.input);
                self.send(text);
            }
            ui.add_space(10.0);
        });
        // 状态行
        if let Some(label) = self.status_label() {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(egui::RichText::new(label).size(11.5).color(th.weak));
            });
        }
        ui.add_space(6.0);
    }

    fn status_label(&self) -> Option<String> {
        if self.recorder.is_some() {
            return Some(format!(
                "● 录音中 {:.0}s（点击「停止」转写，60s 自动停止）",
                self.recorder.as_ref().unwrap().elapsed().as_secs_f32()
            ));
        }
        match self.phase.as_str() {
            "thinking" => Some("思考中…".into()),
            "executing" => Some("执行工具中…".into()),
            "done" => Some("完成".into()),
            _ => None,
        }
    }

    fn settings_ui(&mut self, ctx: &egui::Context) {
        let th = theme(self.dark);
        egui::Window::new(egui::RichText::new("设置").size(16.0))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .resizable(false)
            .collapsible(false)
            .fade_in(false)
            .fade_out(false)
            .frame(
                egui::Frame::default()
                    .fill(th.bg)
                    .stroke(egui::Stroke::new(1.0, th.border))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                let ed = egui::TextEdit::singleline;

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("DeepSeek API Key").color(th.text));
                });
                ui.add(ed(&mut self.cfg.api_key).password(true).desired_width(f32::INFINITY));
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Base URL").color(th.text));
                });
                ui.add(ed(&mut self.cfg.base_url).desired_width(f32::INFINITY));
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("模型").color(th.text));
                });
                ui.add(ed(&mut self.cfg.model).desired_width(f32::INFINITY));
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("语音模型档位").color(th.text));
                    ui.radio_value(&mut self.cfg.voice_model, "fast".to_string(), "fast（默认，更快）");
                    ui.radio_value(&mut self.cfg.voice_model, "quality".to_string(), "quality（更准）");
                });
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("识别语言").color(th.text));
                    ui.radio_value(&mut self.cfg.voice_lang, "zh".to_string(), "中文");
                    ui.radio_value(&mut self.cfg.voice_lang, "en".to_string(), "English");
                    ui.radio_value(&mut self.cfg.voice_lang, "auto".to_string(), "自动");
                });
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut self.cfg.always_on_top, "演示置顶（窗口保持在最前）")
                        .changed()
                    {
                        crate::set_always_on_top_impl(self.hwnd, self.cfg.always_on_top);
                    }
                });
                if ui.checkbox(&mut self.cfg.autostart, "开机自启").changed() {
                    if let Err(e) = crate::set_autostart_impl(self.cfg.autostart) {
                        self.msgs.push(UiMsg::Err(e));
                        self.cfg.autostart = !self.cfg.autostart;
                    }
                }
                ui.label(
                    egui::RichText::new(format!("全局热键：{}（修改热键需重启生效）", self.cfg.hotkey))
                        .size(12.0)
                        .color(th.weak),
                );
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("保存").color(egui::Color32::WHITE),
                            )
                            .fill(th.accent)
                            .corner_radius(8.0)
                            .min_size(egui::vec2(80.0, 28.0)),
                        )
                        .clicked()
                    {
                        self.cfg.theme = if self.dark { "dark".into() } else { "light".into() };
                        match crate::config::save(&self.cfg) {
                            Ok(()) => self.show_settings = false,
                            Err(e) => self.msgs.push(UiMsg::Err(e)),
                        }
                    }
                    if ui.button("取消").clicked() {
                        // 回读放弃改动（置顶等已即时生效项保留）
                        self.cfg = crate::config::load();
                        self.show_settings = false;
                    }
                });
            });
    }

    fn detect_hwnd(&mut self) {
        #[cfg(windows)]
        {
            use windows::core::{w, PCWSTR};
            use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
            unsafe {
                if let Ok(h) = FindWindowW(PCWSTR::null(), w!("Voice Control for Class")) {
                    self.hwnd = h.0 as isize;
                    self.ctl.main_hwnd.store(self.hwnd, Ordering::SeqCst);
                    self.ctl.main_visible.store(true, Ordering::SeqCst);
                    if self.always_on_top {
                        crate::set_always_on_top_impl(self.hwnd, true);
                    }
                    self.hwnd_done = true;
                }
            }
        }
        #[cfg(not(windows))]
        {
            self.hwnd_done = true;
        }
    }
}

/* ---------- App trait ---------- */

impl eframe::App for VccApp {
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
        let th = theme(self.dark);
        // 侧栏
        egui::Panel::left("sidebar")
            .exact_size(261.0)
            .frame(egui::Frame::default().fill(th.bg).inner_margin(0.0))
            .show(ui, |ui| self.sidebar(ui));
        // 输入条
        egui::Panel::bottom("input")
            .frame(egui::Frame::default().fill(th.bg))
            .show(ui, |ui| self.input_bar(ui));
        // 消息流
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(th.bg).inner_margin(egui::Margin::symmetric(0, 8)))
            .show(ui, |ui| self.message_view(ui));

        if self.show_settings {
            self.settings_ui(&ctx);
        }

        // 流式/录音期间持续重绘
        if self.stream_text.is_some() || self.recorder.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}

/* ---------- 工具 ---------- */

fn acquire_busy(state: &SharedState) -> bool {
    state
        .busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// 从运行时历史构建展示列表（最近 40 条；tool/纯 tool_calls 轮过滤）
fn msgs_from_history(state: &SharedState) -> Vec<UiMsg> {
    let hist = state.history.lock().map(|h| h.clone()).unwrap_or_default();
    let start = hist.len().saturating_sub(40);
    let mut out = Vec::new();
    for m in &hist[start..] {
        match m.role.as_str() {
            "user" => {
                if let Some(c) = &m.content {
                    if !c.trim().is_empty() {
                        out.push(UiMsg::User(c.clone()));
                    }
                }
            }
            "assistant" => {
                if let Some(c) = &m.content {
                    if !c.trim().is_empty() {
                        out.push(UiMsg::Ai(c.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// 会话分组：今天 / 昨天 / 7 天内 / 更早（解析「2026年9月6日 14:33」）
fn session_group(updated_at: &str) -> String {
    let d = updated_at.split(' ').next().unwrap_or("");
    let parts: Vec<&str> = d.split(['年', '月', '日']).filter(|s| !s.is_empty()).collect();
    if parts.len() < 3 {
        return "更早".into();
    }
    let (y, mo, da): (i64, u32, u32) = match (
        parts[0].parse::<i64>(),
        parts[1].parse::<u32>(),
        parts[2].parse::<u32>(),
    ) {
        (Ok(a), Ok(b), Ok(c)) => (a, b, c),
        _ => return "更早".into(),
    };
    // 民用日序数（简化：只用于组距比较，不精确处理历法）
    fn day_no(y: i64, m: u32, d: u32) -> i64 {
        let (yy, mm) = if m < 3 { (y - 1, m + 12) } else { (y, m) };
        yy * 372 + mm as i64 * 31 + d as i64
    }
    let now = crate::llm::chrono_now_cn();
    let today = now.split(' ').next().unwrap_or("");
    let tp: Vec<&str> = today.split(['年', '月', '日']).filter(|s| !s.is_empty()).collect();
    if tp.len() >= 3 {
        if let ((Ok(ty), Ok(tmo), Ok(tda)), _) = (
            (
                tp[0].parse::<i64>(),
                tp[1].parse::<u32>(),
                tp[2].parse::<u32>(),
            ),
            (),
        ) {
            let diff = day_no(ty, tmo, tda) - day_no(y, mo, da);
            if diff <= 0 {
                return "今天".into();
            }
            if diff == 1 {
                return "昨天".into();
            }
            if diff < 7 {
                return "7 天内".into();
            }
        }
    }
    "更早".into()
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // 中文主字体（微软雅黑）；粗体族单独注册
    if let Ok(b) = std::fs::read("C:/Windows/Fonts/msyh.ttc") {
        fonts
            .font_data
            .insert("msyh".into(), std::sync::Arc::new(egui::FontData::from_owned(b)));
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .push("msyh".into());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push("msyh".into());
    }
    if let Ok(b) = std::fs::read("C:/Windows/Fonts/msyhbd.ttc") {
        fonts
            .font_data
            .insert("msyhbd".into(), std::sync::Arc::new(egui::FontData::from_owned(b)));
        fonts.families.insert(
            egui::FontFamily::Name("bold".into()),
            vec!["msyhbd".into(), "msyh".into()],
        );
    }
    ctx.set_fonts(fonts);
}

fn apply_visuals(ctx: &egui::Context, dark: bool) {
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let th = theme(dark);
    v.panel_fill = th.bg;
    v.window_fill = th.bg;
    v.extreme_bg_color = th.input_bg;
    v.override_text_color = Some(th.text);
    v.selection.stroke = egui::Stroke::new(1.0, th.accent);
    v.widgets.noninteractive.bg_fill = th.bg;
    v.widgets.inactive.bg_fill = th.input_bg;
    v.widgets.hovered.bg_fill = th.row_hover;
    ctx.set_visuals(v);
}
