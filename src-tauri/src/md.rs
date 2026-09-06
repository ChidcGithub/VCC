/* ---------- 轻量 markdown 渲染（egui LayoutJob） ---------- */
/* 支持流式回答常见形态：代码块 / 标题 / 列表 / **粗体** / `行内代码`。
   不追求完整规范，聊天场景够用；粗体走独立字体族（msyhbd），不依赖 egui 的 strong 语义。 */

use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct MdStyle {
    pub text: egui::Color32,
    pub weak: egui::Color32,
    pub code_bg: egui::Color32,
    pub accent: egui::Color32,
}

pub fn show(ui: &mut egui::Ui, src: &str, st: &MdStyle) {
    let mut in_code = false;
    let mut code = String::new();
    for line in src.lines() {
        if line.trim_start().starts_with("```") {
            if in_code {
                code_panel(ui, &code, st);
                code.clear();
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code.push_str(line);
            code.push('\n');
            continue;
        }
        let t = line.trim_start();
        if let Some(h) = t.strip_prefix("### ") {
            ui.add(egui::Label::new(egui::RichText::new(h).size(14.0).color(st.text).strong()));
        } else if let Some(h) = t.strip_prefix("## ") {
            ui.add(egui::Label::new(egui::RichText::new(h).size(15.0).color(st.text).strong()));
        } else if let Some(h) = t.strip_prefix("# ") {
            ui.add(egui::Label::new(egui::RichText::new(h).size(16.0).color(st.text).strong()));
        } else if t.starts_with("- ") || t.starts_with("* ") {
            ui.horizontal_wrapped(|row| {
                row.label(egui::RichText::new("• ").color(st.accent));
                row.add(egui::Label::new(layout_inline(&t[2..], st)).wrap());
            });
        } else if !t.is_empty() {
            ui.add(egui::Label::new(layout_inline(t, st)).wrap());
        } else {
            ui.add_space(4.0);
        }
    }
    if in_code && !code.trim().is_empty() {
        code_panel(ui, &code, st);
    }
}

fn code_panel(ui: &mut egui::Ui, code: &str, st: &MdStyle) {
    egui::Frame::default()
        .fill(st.code_bg)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(code.trim_end())
                        .font(egui::FontId::monospace(12.5))
                        .color(st.text),
                )
                .wrap(),
            );
        });
    ui.add_space(4.0);
}

/// 行内排版：**粗体** 与 `行内代码` 状态机
fn layout_inline(text: &str, st: &MdStyle) -> egui::text::LayoutJob {
    use egui::{text::LayoutJob, FontFamily, FontId, TextFormat};

    let body = TextFormat::simple(FontId::proportional(14.0), st.text);
    let bold = TextFormat::simple(FontId::new(14.0, FontFamily::Name(Arc::from("bold"))), st.text);
    let code = TextFormat::simple(FontId::monospace(12.5), st.text);

    let mut job = LayoutJob::default();
    let mut seg = String::new();
    let flush = |job: &mut LayoutJob, s: &str, f: &TextFormat| {
        if !s.is_empty() {
            job.append(s, 0.0, f.clone());
        }
    };
    #[derive(PartialEq)]
    enum S {
        Plain,
        Bold,
        Code,
    }
    let mut state = S::Plain;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match state {
            S::Plain => {
                if ch == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
                    flush(&mut job, &seg, &body);
                    seg.clear();
                    state = S::Bold;
                    i += 1; // 跳过第二个 *
                } else if ch == '`' {
                    flush(&mut job, &seg, &body);
                    seg.clear();
                    state = S::Code;
                } else {
                    seg.push(ch);
                }
            }
            S::Bold => {
                if ch == '*' {
                    flush(&mut job, &seg, &bold);
                    seg.clear();
                    state = S::Plain;
                } else {
                    seg.push(ch);
                }
            }
            S::Code => {
                if ch == '`' {
                    flush(&mut job, &seg, &code);
                    seg.clear();
                    state = S::Plain;
                } else {
                    seg.push(ch);
                }
            }
        }
        i += 1;
    }
    // 收尾：未闭合标记按原样吐回
    match state {
        S::Bold => {
            seg.push('*');
            flush(&mut job, &seg, &body);
        }
        S::Code => {
            seg.push('`');
            flush(&mut job, &seg, &body);
        }
        _ => flush(&mut job, &seg, &body),
    }
    job
}
