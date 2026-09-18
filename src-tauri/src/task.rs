//! One foreground operation at a time, cooperative cancellation and explicit approval.
//!
//! Network/recognition waits can be cancelled immediately. A system operation that has
//! already started is allowed to finish: cancellation never pretends to undo side effects.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, Notify};

#[derive(Debug, Default)]
pub struct Cancellation {
    requested: AtomicBool,
    notify: Notify,
}

impl Cancellation {
    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
    pub fn reset(&self) {
        self.requested.store(false, Ordering::SeqCst);
    }
    pub async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            // Register before checking to avoid a lost wake-up between the two.
            notified.as_mut().enable();
            if self.is_requested() {
                return;
            }
            notified.await;
        }
    }
    pub fn check(&self) -> Result<(), String> {
        if self.is_requested() { Err(CANCELLED.into()) } else { Ok(()) }
    }
}

pub const CANCELLED: &str = "已停止。已完成的系统操作不会被撤销。";

/// The lease owns the state rather than borrowing it, so moving it into an async task
/// releases busy on every exit, including panic/unwind and future cancellation.
pub struct RunLease {
    state: crate::SharedState,
    pub generation: u64,
}

impl RunLease {
    pub fn acquire(state: &crate::SharedState) -> Result<Self, String> {
        state.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "上一项任务还在处理，请先停止或等待完成".to_string())?;
        state.cancel.reset();
        let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Self { state: state.clone(), generation })
    }
}

impl Drop for RunLease {
    fn drop(&mut self) {
        self.state.busy.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub struct Approval {
    pub tool: String,
    pub detail: String,
    response: Mutex<Option<oneshot::Sender<bool>>>,
}

impl Approval {
    pub fn new(tool: String, detail: String) -> (Arc<Self>, oneshot::Receiver<bool>) {
        let (tx, rx) = oneshot::channel();
        (Arc::new(Self { tool, detail, response: Mutex::new(Some(tx)) }), rx)
    }
    pub fn respond(&self, approved: bool) {
        if let Ok(mut response) = self.response.lock() {
            if let Some(tx) = response.take() { let _ = tx.send(approved); }
        }
    }
}

/// No LLM-supplied field can bypass the confirmation gate. Only the main UI can
/// complete its one-shot channel. Command blacklists remain defense-in-depth, not a sandbox.
pub fn approval_detail(name: &str, args: &serde_json::Value) -> Option<String> {
    match name {
        "run_command" => Some(format!(
            "运行 PowerShell 命令会影响这台电脑。请检查完整命令；仅本次允许，不会记住授权。\n\n{}",
            args.get("command").and_then(|v| v.as_str()).unwrap_or("（缺少命令）")
        )),
        "write_file" => Some(format!(
            "{}文件：\n{}\n\n以下内容将写入该路径，请确认你有权限且已备份重要数据。\n\n{}",
            if args.get("append").and_then(|v| v.as_bool()).unwrap_or(false) { "追加" } else { "创建或覆盖" },
            args.get("path").and_then(|v| v.as_str()).unwrap_or("（缺少路径）"),
            args.get("content").and_then(|v| v.as_str()).unwrap_or("")
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_releases_on_early_return_and_scopes_generations() {
        let state = Arc::new(crate::VccState::default());
        let first = RunLease::acquire(&state).unwrap();
        assert!(RunLease::acquire(&state).is_err());
        assert_eq!(first.generation, 1);
        drop(first);
        assert!(!state.busy.load(Ordering::SeqCst));
        assert_eq!(RunLease::acquire(&state).unwrap().generation, 2);
    }
    #[test]
    fn lease_releases_on_unwind() {
        let state = Arc::new(crate::VccState::default());
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _lease = RunLease::acquire(&state).unwrap();
            panic!("test unwind");
        }));
        assert!(!state.busy.load(Ordering::SeqCst));
    }
    #[tokio::test]
    async fn cancellation_before_wait_is_not_lost() {
        let cancel = Cancellation::default();
        cancel.request();
        tokio::time::timeout(std::time::Duration::from_millis(50), cancel.cancelled()).await.unwrap();
        assert!(cancel.check().is_err());
        cancel.reset();
        assert!(cancel.check().is_ok());
    }
    #[tokio::test]
    async fn approval_is_single_use_and_deny_by_default() {
        let (approval, rx) = Approval::new("write_file".into(), "test".into());
        approval.respond(false);
        approval.respond(true);
        assert!(!rx.await.unwrap());
        let (approval, rx) = Approval::new("run_command".into(), "test".into());
        drop(approval);
        assert!(rx.await.is_err());
    }
    #[test]
    fn model_cannot_self_approve() {
        assert!(approval_detail("run_command", &serde_json::json!({"approved":true,"command":"Get-Date"})).is_some());
        assert!(approval_detail("write_file", &serde_json::json!({"path":"a.txt"})).is_some());
        assert!(approval_detail("get_volume", &serde_json::json!({})).is_none());
    }
}
