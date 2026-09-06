/* ---------- UI ↔ 后端事件总线：替代 Tauri 的 emit/listen ---------- */
/* 后端（llm agent / 托盘 / 热键）通过 EventTx 推 UiEvent；
   UI 线程（eframe update）每帧 poll；
   UI → 泵线程（overlay/悬浮窗）走单独的 PumpMsg 通道。
   主窗控制（呼出/置顶/退出）走 WinCtl 原子旗标，泵线程直接 Win32 执行。 */

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// 悬浮窗步骤卡里的一行
#[derive(Debug, Clone)]
pub struct Step {
    pub label: String,
    pub status: String, // running / done / fail
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    /// 流式回答开始（开新气泡）
    ChatStart,
    ChatDelta(String),
    /// 流结束（渲染 markdown + 复位）
    ChatEnd,
    /// 成品气泡（err 错误 / 纯工具轮的完成提示）
    ChatBubble { role: &'static str, text: String },
    ToolStart { id: String, label: String },
    ToolDone { id: String, ok: bool },
    /// 相变：idle / summoned / thinking / listening / executing / done
    Phase(String),
    Float { mode: String, steps: Vec<Step>, text: String },
    /// 热键/托盘/二次启动 → 主窗已呼出
    Invoked,
    /// 语音转写完成（自动发送或回填输入框）
    Transcribed(Result<String, String>),
}

#[derive(Clone)]
pub struct EventTx(Sender<UiEvent>);

impl EventTx {
    pub fn send(&self, ev: UiEvent) {
        // 接收端活着（UI 线程）才有人收；UI 没起来时静默丢弃
        let _ = self.0.send(ev);
    }
}

pub fn event_channel() -> (EventTx, Receiver<UiEvent>) {
    let (tx, rx) = channel();
    (EventTx(tx), rx)
}

/* ---------- 泵线程消息（UI → overlay/悬浮窗/音量） ---------- */

#[derive(Debug, Clone)]
pub enum PumpMsg {
    Phase(String),
    Float { mode: String, steps: Vec<Step>, text: String },
    /// 麦克风实时电平 0..1（跑马灯呼吸强度）
    Level(f32),
}

pub fn pump_channel() -> (Sender<PumpMsg>, Receiver<PumpMsg>) {
    channel()
}

/* ---------- 主窗控制旗标（跨线程窗口操作） ---------- */

#[derive(Default)]
pub struct WinCtl {
    /// 泵线程 → 主窗：请求显示/聚焦（热键 toggle 的"显示"半边）
    pub show_main: AtomicBool,
    /// 主窗 HWND（UI 线程首帧写入；0 = 未就绪）
    pub main_hwnd: AtomicIsize,
    /// 主窗当前是否可见（UI 线程维护，泵线程 toggle 用）
    pub main_visible: AtomicBool,
    /// 泵线程 → 全体：退出
    pub exit: AtomicBool,
}

impl WinCtl {
    pub fn request_show(&self) {
        self.show_main.store(true, Ordering::SeqCst);
    }
    pub fn hwnd(&self) -> isize {
        self.main_hwnd.load(Ordering::SeqCst)
    }
    pub fn visible(&self) -> bool {
        self.main_visible.load(Ordering::SeqCst)
    }
}

pub type SharedWinCtl = Arc<WinCtl>;
