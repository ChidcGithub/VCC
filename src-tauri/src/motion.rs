/* ---------- 小布 Next 动效体系（COUI + Material 3） ----------
   来源：《小布Next_UI动效复用规范_Rust.md》（com.oplus.claw 17.0.72 提取）。
   - 时长阶梯 = M3 duration token（150/250/350/400ms…）
   - 曲线 = COUI 14 条 + M3 9 条 cubic-bezier 精确控制点（硬编码）
   - 规律：退出快于进入（250 进 / 150 出）；位移用百分比；alpha 与位移解耦
   - 时钟 = Instant 单调时钟（不累加 dt，掉帧不漂移）
   - 中断 = 新动画从当前值起步（retarget），不做简单取反                */

use std::time::{Duration, Instant};

/* ---------- 时长 token（M3 duration） ---------- */

pub const D_SHORT3: u64 = 150; // 退出、对话框关闭
pub const D_MEDIUM1: u64 = 250; // 居中对话框进入
pub const D_MEDIUM3: u64 = 350; // 面板进入、侧滑
pub const D_MEDIUM4: u64 = 400; // BottomSheet 进入

/* 场景参数（规范 §4.3 实测值） */
pub const DIALOG_IN_MS: u64 = D_MEDIUM1; // 居中对话框进 250ms（scale 0.8→1 + alpha 0→1）
pub const DIALOG_OUT_MS: u64 = D_SHORT3; // 居中对话框出 150ms（alpha 1→0）
pub const PANEL_IN_MS: u64 = D_MEDIUM3; // 面板 Fragment 进 350ms（scale 0.9→1 + alpha 0→1）
pub const PANEL_OUT_MS: u64 = D_SHORT3; // 面板 Fragment 出 150ms
pub const BUBBLE_MS: u64 = 220; // 气泡「从底部生长淡入」位移段（alpha 段 150ms）
pub const INTERACT_MS: u64 = 150; // 交互过渡（hover/enable）
pub const CURSOR_BREATH_S: f32 = 1.0; // 流式光标呼吸周期

/* ---------- Cubic-Bezier 求值（Newton-Raphson，规范 §7.2） ---------- */

#[derive(Clone, Copy, Debug)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl CubicBezier {
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    #[inline]
    fn bezier(t: f32, p1: f32, p2: f32) -> f32 {
        let u = 1.0 - t;
        3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
    }
    #[inline]
    fn bezier_dt(t: f32, p1: f32, p2: f32) -> f32 {
        let u = 1.0 - t;
        3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
    }

    /// x: 归一化时间进度 → y: 归一化输出进度
    pub fn eval(&self, x: f32) -> f32 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        let mut t = x;
        for _ in 0..8 {
            let err = Self::bezier(t, self.x1, self.x2) - x;
            if err.abs() < 1e-5 {
                break;
            }
            let d = Self::bezier_dt(t, self.x1, self.x2);
            if d.abs() < 1e-6 {
                break;
            }
            t -= err / d;
            t = t.clamp(0.0, 1.0);
        }
        Self::bezier(t, self.y1, self.y2)
    }
}

/// Android accelerate-decelerate（COUI push up/down 用）
pub fn android_acc_dec(x: f32) -> f32 {
    ((x + 1.0) * std::f32::consts::PI).cos() / 2.0 + 0.5
}

/* ---------- COUI 曲线（照搬 APK，规范 §4.2） ---------- */

pub mod curve {
    use super::CubicBezier as B;

    pub const COUI_EASE: B = B::new(0.33, 0.00, 0.67, 1.00);
    pub const COUI_EASE_IN: B = B::new(0.00, 0.00, 0.10, 1.00);
    pub const COUI_EASE_OUT: B = B::new(0.30, 0.00, 1.00, 1.00);
    pub const COUI_EASE_MOVE: B = B::new(0.30, 0.00, 0.10, 1.00);
    pub const COUI_OPACITY_INOUT: B = B::new(0.33, 0.00, 0.67, 1.00);
    pub const COUI_OPEN_SLIDE: B = B::new(0.25, 0.10, 0.30, 1.00);
    pub const COUI_OPEN_SLIDE_IN: B = B::new(0.30, 0.10, 0.30, 1.00);
    pub const COUI_OPEN_SLIDE_OUT: B = B::new(0.30, 0.15, 0.30, 1.00);
    pub const COUI_CLOSE_SLIDE_IN: B = B::new(0.30, 0.26, 0.40, 1.00);
    pub const COUI_CLOSE_SLIDE_OUT: B = B::new(0.25, 0.10, 0.30, 1.00);
    pub const COUI_TASK_SCALE_UP: B = B::new(0.15, 0.00, 0.50, 1.00);
    pub const COUI_TASK_SCALE_DOWN: B = B::new(0.33, 0.00, 0.67, 1.00);
    /// 任务卡滑动（轻微回弹感）——气泡入场用它
    pub const COUI_TASK_SLIDE: B = B::new(0.40, 0.40, 0.08, 1.00);

    /* Material 3 */
    pub const M3_STANDARD: B = B::new(0.20, 0.00, 0.00, 1.00);
    pub const M3_STD_ACCELERATE: B = B::new(0.30, 0.00, 1.00, 1.00);
    pub const M3_STD_DECELERATE: B = B::new(0.00, 0.00, 0.00, 1.00);
    pub const M3_EMPH_ACCELERATE: B = B::new(0.30, 0.00, 0.80, 0.20); // 退出
    pub const M3_EMPH_DECELERATE: B = B::new(0.10, 0.70, 0.10, 1.00); // 进入
    pub const LEGACY_ACCELERATE: B = B::new(0.40, 0.00, 1.00, 1.00);
    pub const LEGACY_DECELERATE: B = B::new(0.00, 0.00, 0.20, 1.00);
}

/* ---------- M3 emphasized path 缓动（两段贝塞尔，规范 §7.3） ----------
   path: M 0,0 C 0.05,0 0.133333,0.06 0.166666,0.4 C 0.208333,0.82 0.25,1 1,1 */

pub fn m3_emphasized(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    if x <= 0.166666 {
        cubic_y_for_x(
            x,
            (0.0, 0.0),
            (0.05, 0.0),
            (0.133333, 0.06),
            (0.166666, 0.4),
        )
    } else {
        cubic_y_for_x(
            x,
            (0.166666, 0.4),
            (0.208333, 0.82),
            (0.25, 1.0),
            (1.0, 1.0),
        )
    }
}

/// 在单个三次贝塞尔段内，给定 x 反解 t 再取 y
fn cubic_y_for_x(
    x: f32,
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
) -> f32 {
    let at = |t: f32, a: f32, b: f32, c: f32, d: f32| {
        let u = 1.0 - t;
        u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
    };
    let dt = |t: f32, a: f32, b: f32, c: f32, d: f32| {
        let u = 1.0 - t;
        3.0 * u * u * (b - a) + 6.0 * u * t * (c - b) + 3.0 * t * t * (d - c)
    };
    let mut t = ((x - p0.0) / (p3.0 - p0.0).max(f32::EPSILON)).clamp(0.0, 1.0);
    for _ in 0..8 {
        let err = at(t, p0.0, p1.0, p2.0, p3.0) - x;
        if err.abs() < 1e-5 {
            break;
        }
        let d = dt(t, p0.0, p1.0, p2.0, p3.0);
        if d.abs() < 1e-6 {
            break;
        }
        t -= err / d;
        t = t.clamp(0.0, 1.0);
    }
    at(t, p0.1, p1.1, p2.1, p3.1)
}

/* ---------- 动画时钟（规范 §7.4） ---------- */

pub struct Anim {
    start: Instant,
    dur: Duration,
    curve: CubicBezier,
    pub from: f32,
    pub to: f32,
}

impl Anim {
    pub fn new(dur_ms: u64, curve: CubicBezier, from: f32, to: f32) -> Self {
        Self {
            start: Instant::now(),
            dur: Duration::from_millis(dur_ms),
            curve,
            from,
            to,
        }
    }

    /// 缓动后的进度 ∈ [0,1]
    pub fn progress(&self) -> f32 {
        let t = (self.start.elapsed().as_secs_f32() / self.dur.as_secs_f32()).clamp(0.0, 1.0);
        self.curve.eval(t)
    }

    pub fn value(&self) -> f32 {
        self.from + (self.to - self.from) * self.progress()
    }

    pub fn done(&self) -> bool {
        self.start.elapsed() >= self.dur
    }

    /// 中断时反向：新动画从当前值起步（COUI 做法），不是简单取反
    pub fn retarget(&mut self, to: f32, dur_ms: u64) {
        let cur = self.value();
        *self = Anim::new(dur_ms, self.curve, cur, to);
    }
}

/* ---------- 绘制工具 ---------- */

/// 颜色线性插值（0=a, 1=b）
pub fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgba_unmultiplied(
        f(a.r(), b.r()),
        f(a.g(), b.g()),
        f(a.b(), b.b()),
        f(a.a(), b.a()),
    )
}

/// 半透明纯色（用于 scrim 遮罩 / 从背景浮现）
pub fn scrim(c: egui::Color32, alpha: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (255.0 * alpha.clamp(0.0, 1.0)) as u8)
}

/// COUI loading：单层椭圆旋转的纯 Rust 替代（规范 §7.5）
/// 实测 Lottie 参数：84×84、60fps、0–76 帧 → 1.27s 一圈、3/4 圆弧。
/// 文档示例的 `t * TAU * 1.27`（每秒 1.27 圈）与「周期 1.27s」矛盾，按周期取 1/1.27。
pub fn draw_spinner(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    width: f32,
    color: egui::Color32,
    t_sec: f32,
) {
    let angle = t_sec * std::f32::consts::TAU / 1.27;
    let sweep = std::f32::consts::PI * 0.75; // 3/4 圆弧
    let pts: Vec<egui::Pos2> = (0..=24)
        .map(|i| {
            let a = angle + sweep * (i as f32 / 24.0);
            center + egui::vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(
        pts,
        egui::Stroke::new(width, color),
    ));
    // egui 无线帽控制：两端补小圆点近似圆头
    for a in [angle, angle + sweep] {
        painter.circle_filled(center + egui::vec2(a.cos(), a.sin()) * radius, width * 0.5, color);
    }
}

/* ---------- 每帧驱动 ---------- */

/// 动画/呼吸未结束时由调用方请求重绘（16ms ≈ 60fps）
pub fn repaint_tick(ctx: &egui::Context) {
    ctx.request_repaint_after(Duration::from_millis(16));
}
