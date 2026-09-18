# -*- coding: utf-8 -*-
"""动效接入修复：编译错误 + memory 新 API 适配 + reduce_motion 接入"""
import sys

PATH = r"D:\My things\Learn\高二\VCC\src-tauri\src\ui.rs"

REPL = []

# F1 use Instant
REPL.append((
"""use std::sync::mpsc::{Receiver, Sender};""",
"""use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;""", 1))

# F2 settings_anim 推进块（借用修复：不能在 &self.settings_anim 中写同字段）
REPL.append((
"""        // ---- 设置浮层动画推进（250ms 进 / 150ms 出，出完才真正关） ----
        if let Some((opening, a)) = &self.settings_anim {
            if a.done() {
                if !*opening {
                    self.show_settings = false;
                }
                self.settings_anim = None;
            }
        }""",
"""        // ---- 设置浮层动画推进（250ms 进 / 150ms 出，出完才真正关） ----
        let settings_closing_done = self
            .settings_anim
            .as_ref()
            .is_some_and(|(opening, a)| a.done() && !*opening);
        if settings_closing_done {
            self.show_settings = false;
            self.settings_anim = None;
        }""", 1))

# F3 ESC：reduce_motion 直接隐藏，否则淡出
REPL.append((
"""        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.fade_out.is_none() {
            self.fade_out = Some(Anim::new(
                motion::PANEL_OUT_MS,
                motion::curve::M3_EMPH_ACCELERATE,
                0.0,
                1.0,
            ));
        }""",
"""        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.cfg.reduce_motion {
                #[cfg(windows)]
                unsafe {
                    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
                    if self.hwnd != 0 {
                        let _ = ShowWindow(
                            windows::Win32::Foundation::HWND(self.hwnd as _),
                            SW_HIDE,
                        );
                    }
                }
                self.ctl.main_visible.store(false, Ordering::SeqCst);
            } else if self.fade_out.is_none() {
                self.fade_out = Some(Anim::new(
                    motion::PANEL_OUT_MS,
                    motion::curve::M3_EMPH_ACCELERATE,
                    0.0,
                    1.0,
                ));
            }
        }""", 1))

# F4 intro：reduce_motion 不播入场
REPL.append((
"""        if vis && !self.last_visible {
            self.intro = Some(Anim::new(""",
"""        if vis && !self.last_visible && !self.cfg.reduce_motion {
            self.intro = Some(Anim::new(""", 1))

# F5 open_settings reduce_motion
REPL.append((
"""        self.show_settings = true;
        self.settings_anim = Some((
            true,
            Anim::new(motion::DIALOG_IN_MS, motion::curve::M3_EMPH_DECELERATE, 0.0, 1.0),
        ));""",
"""        self.show_settings = true;
        if self.cfg.reduce_motion {
            return;
        }
        self.settings_anim = Some((
            true,
            Anim::new(motion::DIALOG_IN_MS, motion::curve::M3_EMPH_DECELERATE, 0.0, 1.0),
        ));""", 1))

# F6 close_settings reduce_motion
REPL.append((
"""        if let Some((opening, _)) = &self.settings_anim {
            if !*opening {
                return;
            }
        }
        self.settings_anim = Some((
            false,
            Anim::new(motion::DIALOG_OUT_MS, motion::curve::M3_EMPH_ACCELERATE, 0.0, 1.0),
        ));""",
"""        if let Some((opening, _)) = &self.settings_anim {
            if !*opening {
                return;
            }
        }
        if self.cfg.reduce_motion {
            self.show_settings = false;
            return;
        }
        self.settings_anim = Some((
            false,
            Anim::new(motion::DIALOG_OUT_MS, motion::curve::M3_EMPH_ACCELERATE, 0.0, 1.0),
        ));""", 1))

# F7 push_msg：reduce_motion 不记入场
REPL.append((
"""    fn push_msg(&mut self, m: UiMsg) {
        self.msgs.push(m);
        let idx = self.msgs.len() - 1;
        self.entering.push((idx, Instant::now()));""",
"""    fn push_msg(&mut self, m: UiMsg) {
        self.msgs.push(m);
        if self.cfg.reduce_motion {
            return;
        }
        let idx = self.msgs.len() - 1;
        self.entering.push((idx, Instant::now()));""", 1))

# F8 new_chat：new_session Result 适配
REPL.append((
"""        let id = crate::memory::new_session();
        if let Ok(mut c) = self.state.current.lock() {
            *c = id.clone();
        }
        self.current_sid = id;
        self.msgs.clear();""",
"""        let id = match crate::memory::new_session() {
            Ok(id) => id,
            Err(e) => {
                self.push_msg(UiMsg::Err(e));
                self.refresh_sessions();
                return;
            }
        };
        if let Ok(mut c) = self.state.current.lock() {
            *c = id.clone();
        }
        self.current_sid = id;
        self.msgs.clear();""", 1))

# F9 delete_session：Result 适配
REPL.append((
"""        let new_cur = crate::memory::delete_session(id);
        let final_id = if new_cur.is_empty() {
            crate::memory::new_session()
        } else {
            new_cur
        };""",
"""        let final_id = match crate::memory::delete_session(id) {
            Ok(next) if !next.is_empty() => next,
            _ => match crate::memory::new_session() {
                Ok(n) => n,
                Err(e) => {
                    self.push_msg(UiMsg::Err(e));
                    self.refresh_sessions();
                    return;
                }
            },
        };""", 1))

# F10 commit_rename：rename_session Result
REPL.append((
"""            if !t.is_empty() {
                crate::memory::rename_session(&id, &t);
            }""",
"""            if !t.is_empty() {
                if let Err(e) = crate::memory::rename_session(&id, &t) {
                    self.push_msg(UiMsg::Err(e));
                }
            }""", 1))

# F11 save_history 忽略错误
REPL.append((
"""            crate::memory::save_history(&hist);""",
"""            let _ = crate::memory::save_history(&hist);""", 1))

# F12 screen_rect → viewport_rect（3 处）
REPL.append((
"""ctx.screen_rect()""",
"""ctx.viewport_rect()""", 3))

# F13 Response.layer_id 字段
REPL.append((
"""                    r.response.layer_id(),""",
"""                    r.response.layer_id,""", 1))

# F14 style_mut → all_styles_mut（双主题统一）
REPL.append((
"""    ctx.style_mut(|style| style.animation_time = 0.15);""",
"""    ctx.all_styles_mut(|style| style.animation_time = 0.15);""", 1))

# F15 光标呼吸 reduce_motion 恒定
REPL.append((
"""                let cursor_color = motion::scrim(th.accent, 0.35 + 0.65 * breath);""",
"""                let cursor_color = if self.cfg.reduce_motion {
                    th.accent
                } else {
                    motion::scrim(th.accent, 0.35 + 0.65 * breath)
                };""", 1))

# F16 设置面板：减少动态效果开关
REPL.append((
"""                if ui.checkbox(&mut self.cfg.autostart, "开机自启").changed() {""",
"""                if ui
                    .checkbox(&mut self.cfg.reduce_motion, "减少动态效果")
                    .changed()
                {
                    let _ = crate::config::save(&self.cfg);
                }
                if ui.checkbox(&mut self.cfg.autostart, "开机自启").changed() {""", 1))


def main():
    with open(PATH, "r", encoding="utf-8") as f:
        src = f.read()
    fails = []
    for i, (old, new, want) in enumerate(REPL, 1):
        n = src.count(old)
        if n != want:
            fails.append((i, n, want))
            continue
        src = src.replace(old, new)
    if fails:
        for i, n, want in fails:
            print(f"FAIL patch#{i} hits={n} (期望 {want})")
        sys.exit(1)
    with open(PATH, "w", encoding="utf-8", newline="") as f:
        f.write(src)
    print(f"OK: {len(REPL)} patches applied")


if __name__ == "__main__":
    main()
