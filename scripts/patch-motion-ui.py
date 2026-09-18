# -*- coding: utf-8 -*-
"""小布 Next 动效接入 ui.rs —— 剩余修改（Edit 假成功，改走 patch 脚本）"""
import sys, ast

PATH = r"D:\My things\Learn\高二\VCC\src-tauri\src\ui.rs"

REPL = []

# (1) reload_display 加 entering.clear
REPL.append((
"""    fn reload_display(&mut self) {
        self.msgs = msgs_from_history(&self.state);
        self.stream_text = None;
    }""",
"""    fn reload_display(&mut self) {
        self.msgs = msgs_from_history(&self.state);
        self.stream_text = None;
        self.entering.clear();
    }"""))

# (2) settings_ui：保存/取消 → close_settings；show 捕获返回值加 scale transform
REPL.append((
"""                        self.cfg.theme = if self.dark { "dark".into() } else { "light".into() };
                        match crate::config::save(&self.cfg) {
                            Ok(()) => self.show_settings = false,
                            Err(e) => self.push_msg(UiMsg::Err(e)),
                        }
                    }
                    if ui.button("取消").clicked() {
                        // 回读放弃改动（置顶等已即时生效项保留）
                        self.cfg = crate::config::load();
                        self.show_settings = false;
                    }
                });
            });
    }""",
"""                        self.cfg.theme = if self.dark { "dark".into() } else { "light".into() };
                        match crate::config::save(&self.cfg) {
                            Ok(()) => self.close_settings(),
                            Err(e) => self.push_msg(UiMsg::Err(e)),
                        }
                    }
                    if ui.button("取消").clicked() {
                        // 回读放弃改动（置顶等已即时生效项保留）
                        self.cfg = crate::config::load();
                        self.close_settings();
                    }
                });
            })
            .map(|r| {
                // scale 动画：围绕窗口中心缩放（渲染 + 命中测试同变换）
                let c = r.response.rect.center();
                ctx.set_transform_layer(
                    r.response.layer_id(),
                    egui::emath::TSTransform {
                        scaling: scale,
                        translation: c.to_vec2() - scale * c.to_vec2(),
                    },
                );
            });
        motion::repaint_tick(ctx); // 对话框动画/呼吸驱动
    }"""))

# (3) message_view：boot/now + enumerate + 气泡入场位移
REPL.append((
"""    fn message_view(&mut self, ui: &mut egui::Ui) {
        let th = theme(self.dark);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(self.stream_text.is_some())
            .show(ui, |ui| {
                let width = ui.available_width();
                for m in &self.msgs {
                    match m {""",
"""    fn message_view(&mut self, ui: &mut egui::Ui) {
        let th = theme(self.dark);
        let boot = self.boot; // Instant: Copy
        let now = Instant::now();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(self.stream_text.is_some())
            .show(ui, |ui| {
                let width = ui.available_width();
                for (idx, m) in self.msgs.iter().enumerate() {
                    // COUI 气泡入场：从底部生长淡入（位移 220ms，TASK_SLIDE 轻回弹）
                    let dy = self
                        .entering
                        .iter()
                        .find(|(i, t)| {
                            *i == idx && now.duration_since(*t).as_millis() < motion::BUBBLE_MS as u128
                        })
                        .map(|(_, t)| {
                            let pr = now.duration_since(*t).as_secs_f32()
                                / (motion::BUBBLE_MS as f32 / 1000.0);
                            10.0 * (1.0 - motion::curve::COUI_TASK_SLIDE.eval(pr.min(1.0)))
                        })
                        .unwrap_or(0.0);
                    if dy > 0.5 {
                        ui.add_space(dy);
                    }
                    match m {"""))

# (4) 工具行：egui Spinner → COUI 手绘 spinner
REPL.append((
"""                                match done {
                                    None => {
                                        ui.add(egui::Spinner::new().size(12.0).color(th.accent));
                                        ui.label(egui::RichText::new(label).color(th.weak).size(12.5));
                                    }""",
"""                                match done {
                                    None => {
                                        // COUI loading：1.27s/圈 3/4 圆弧手绘 spinner
                                        let (rect, _) = ui.allocate_exact_size(
                                            egui::vec2(14.0, 14.0),
                                            egui::Sense::hover(),
                                        );
                                        motion::draw_spinner(
                                            ui.painter(),
                                            rect.center(),
                                            6.0,
                                            2.2,
                                            th.accent,
                                            boot.elapsed().as_secs_f32(),
                                        );
                                        ui.label(egui::RichText::new(label).color(th.weak).size(12.5));
                                    }"""))

# (5) 流式气泡：空文本换 spinner + 光标呼吸
REPL.append((
"""                // 流式气泡
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
                }""",
"""                // 流式气泡（光标呼吸：1s 周期 COUI 淡入淡出曲线）
                let breath = 0.5
                    - 0.5 * (boot.elapsed().as_secs_f32() * std::f32::consts::TAU
                        / motion::CURSOR_BREATH_S)
                        .cos();
                let cursor_color = motion::scrim(th.accent, 0.35 + 0.65 * breath);
                if let Some(s) = &self.stream_text {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(width * 0.86);
                            if s.is_empty() {
                                // COUI spinner 代替省略号
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(14.0, 14.0),
                                    egui::Sense::hover(),
                                );
                                motion::draw_spinner(
                                    ui.painter(),
                                    rect.center(),
                                    6.0,
                                    2.2,
                                    th.accent,
                                    boot.elapsed().as_secs_f32(),
                                );
                            } else {
                                crate::md::show(ui, s, &th.md());
                                ui.label(egui::RichText::new("▌").color(cursor_color).size(13.0));
                            }
                        });
                    });
                }"""))

# (6) message_view 结尾：entering 过期清理（放闭包外避免 mut 捕获）
REPL.append((
"""            });
    }

    fn input_bar(&mut self, ui: &mut egui::Ui) {""",
"""            });
        // 入场完成的气泡索引清理（闭包外，避免闭包 mut 捕获）
        self.entering
            .retain(|(_, t)| now.duration_since(*t).as_millis() < (motion::BUBBLE_MS as u128) + 50);
    }

    fn input_bar(&mut self, ui: &mut egui::Ui) {"""))

# (7) 发送按钮：enable 状态颜色过渡 150ms
REPL.append((
"""            // 发送按钮
            let can_send = !self.input.trim().is_empty();
            let send_btn = egui::Button::new(
                egui::RichText::new("发送").color(if can_send { egui::Color32::WHITE } else { th.weak }),
            )
            .fill(if can_send { th.accent } else { th.input_bg })
            .corner_radius(100.0)
            .min_size(egui::vec2(64.0, 30.0));""",
"""            // 发送按钮（enable 颜色过渡：150ms M3_STANDARD）
            let can_send = !self.input.trim().is_empty();
            let t_ready = ui.ctx().animate_value_with_time(
                egui::Id::new("send_ready"),
                if can_send { 1.0 } else { 0.0 },
                motion::INTERACT_MS as f32 / 1000.0,
            );
            let send_btn = egui::Button::new(egui::RichText::new("发送").color(motion::lerp_color(
                th.weak,
                egui::Color32::WHITE,
                t_ready,
            )))
            .fill(motion::lerp_color(th.input_bg, th.accent, t_ready))
            .corner_radius(100.0)
            .min_size(egui::vec2(64.0, 30.0));"""))

# (8) apply_visuals：egui 内置动画时长对齐 COUI 交互时长
REPL.append((
"""    v.widgets.inactive.bg_fill = th.input_bg;
    v.widgets.hovered.bg_fill = th.row_hover;
    ctx.set_visuals(v);
}""",
"""    v.widgets.inactive.bg_fill = th.input_bg;
    v.widgets.hovered.bg_fill = th.row_hover;
    ctx.set_visuals(v);
    // COUI 交互时长：egui 内置动画统一 150ms
    ctx.style_mut(|style| style.animation_time = 0.15);
}"""))


def main():
    with open(PATH, "r", encoding="utf-8") as f:
        src = f.read()
    fails = []
    for i, (old, new) in enumerate(REPL, 1):
        n = src.count(old)
        if n != 1:
            fails.append((i, n))
            continue
        src = src.replace(old, new, 1)
    if fails:
        for i, n in fails:
            print(f"FAIL patch#{i} hits={n} (期望 1)")
        sys.exit(1)
    with open(PATH, "w", encoding="utf-8", newline="") as f:
        f.write(src)
    print(f"OK: {len(REPL)} patches applied")


if __name__ == "__main__":
    ast.parse  # noqa: B018（占位，保持与历史脚本一致的预检习惯）
    main()
