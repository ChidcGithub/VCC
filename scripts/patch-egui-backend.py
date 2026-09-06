# -*- coding: utf-8 -*-
"""egui 迁移 patch（第二轮：只补未生效的 8 处 llm.rs + 全部 voice.rs + tools.rs）。
每个补丁必须精确命中一次，否则报错退出（绝不静默跳过）。"""
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
            print("---- old 片段头 80 字符 ----")
            print(old[:80].replace("\n", "\\n"))
            sys.exit(1)
        src = src.replace(old, new)
    with open(p, "w", encoding="utf-8", newline="") as f:
        f.write(src)
    print(f"[OK] {path}: {len(patches)} patches")

# ============ llm.rs（剩余 8 处） ============
LLM = [
# import
("""use crate::{config::Config, tools};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};""",
"""use crate::bus::{EventTx, Step, UiEvent};
use crate::config::Config;
use crate::tools;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};"""),

# stream signature
("""async fn chat_completion_stream(
    cfg: &Config,
    messages: &[ChatMessage],
    app: &AppHandle,
) -> Result<ChatMessage, String> {""",
"""async fn chat_completion_stream(
    cfg: &Config,
    messages: &[ChatMessage],
    ev: &EventTx,
) -> Result<ChatMessage, String> {"""),

# final flush
("""    if !delta_buf.is_empty() {
        let _ = app.emit("vcc://chat-delta", json!({"text": delta_buf}));
    }""",
"""    if !delta_buf.is_empty() {
        ev.send(UiEvent::ChatDelta(delta_buf.clone()));
    }"""),

# run_agent head
("""pub async fn run_agent(app: &AppHandle, text: String) -> Result<(), String> {
    let cfg = crate::config::load(app);
    let memory = crate::memory::load_memory(app);

    let history = {
        let state = app.state::<crate::AppState>();
        let mut hist = state.history.lock().map_err(|_| "历史锁错误")?;""",
"""pub async fn run_agent(state: &crate::VccState, ev: &EventTx, text: String) -> Result<(), String> {
    let cfg = crate::config::load();
    let memory = crate::memory::load_memory();

    let history = {
        let mut hist = state.history.lock().map_err(|_| "历史锁错误")?;"""),

# summarize spawn
("""            if !dropped.is_empty() {
                let app2 = app.clone();
                tauri::async_runtime::spawn(async move {
                    crate::memory::summarize_into_memory(app2, dropped).await;
                });
            }""",
"""            if !dropped.is_empty() {
                tokio::spawn(async move {
                    crate::memory::summarize_into_memory(dropped).await;
                });
            }"""),

# thinking phase
("""    let _ = app.emit("vcc://phase", json!({"phase": "thinking"}));""",
"""    ev.send(UiEvent::Phase("thinking".into()));"""),

# tool loop
("""        let msg = chat_completion_stream(&cfg, &messages, app).await?;

        if let Some(tcs) = &msg.tool_calls {
            if !tcs.is_empty() {
                messages.push(msg.clone());
                for tc in tcs {
                    let label = tool_label(&tc.function.name, &serde_json::from_str::<Value>(&tc.function.arguments).unwrap_or_else(|_| json!({})));
                    steps.push(json!({"label": label, "status": "running"}));

                    let _ = app.emit("vcc://phase", json!({"phase": "executing"}));
                    let _ = app.emit("vcc://tool", json!({"id": tc.id, "label": label}));
                    let _ = app.emit("vcc://float", json!({"mode": "show", "steps": steps, "state": "执行中"}));""",
"""        let msg = chat_completion_stream(&cfg, &messages, ev).await?;

        if let Some(tcs) = &msg.tool_calls {
            if !tcs.is_empty() {
                messages.push(msg.clone());
                for tc in tcs {
                    let label = tool_label(&tc.function.name, &serde_json::from_str::<Value>(&tc.function.arguments).unwrap_or_else(|_| json!({})));
                    steps.push(Step { label, status: "running".into() });

                    ev.send(UiEvent::Phase("executing".into()));
                    ev.send(UiEvent::ToolStart { id: tc.id.clone(), label: steps.last().unwrap().label.clone() });
                    ev.send(UiEvent::Float { mode: "show".into(), steps: steps.clone(), text: String::new() });"""),

# tool done
("""                    steps.last_mut().unwrap()["status"] = json!(if ok { "done" } else { "fail" });
                    let _ = app.emit("vcc://tool-done", json!({"id": tc.id, "ok": ok}));
                    let _ = app.emit("vcc://float", json!({"mode": "show", "steps": steps, "state": "执行中"}));""",
"""                    steps.last_mut().unwrap().status = if ok { "done" } else { "fail" }.into();
                    ev.send(UiEvent::ToolDone { id: tc.id.clone(), ok });
                    ev.send(UiEvent::Float { mode: "show".into(), steps: steps.clone(), text: String::new() });"""),
]

# ============ voice.rs ============
VOICE = [
("""use base64::Engine;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU16, Ordering};
use tauri::Manager;
use tokio::process::Command;""",
"""use base64::Engine;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU16, Ordering};
use tokio::process::Command;"""),

("""fn server_info_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("server.json"))
}

fn read_server_info(app: &tauri::AppHandle) -> Option<ServerInfo> {
    let p = server_info_path(app)?;
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

fn write_server_info(app: &tauri::AppHandle, info: &ServerInfo) {
    if let Some(p) = server_info_path(app) {
        if let Ok(j) = serde_json::to_string(info) {
            let _ = std::fs::write(p, j);
        }
    }
}""",
"""fn server_info_path() -> Option<PathBuf> {
    Some(crate::config::data_dir().join("server.json"))
}

fn read_server_info() -> Option<ServerInfo> {
    let p = server_info_path()?;
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

fn write_server_info(info: &ServerInfo) {
    if let Some(p) = server_info_path() {
        if let Ok(j) = serde_json::to_string(info) {
            let _ = std::fs::write(p, j);
        }
    }
}"""),

("""    if let Some(app) = crate::APP_HANDLE.get() {
        if let Some(info) = read_server_info(app) {
            if info.model == model_name && info.lang == lang && probe(info.port) {
                SERVER_PORT.store(info.port, Ordering::Relaxed);
                touch_server();
                return Ok(info.port);
            }
            // 存在但不健康或档位不符 → 清理
            kill_pid(info.pid);
        }
    }""",
"""    if let Some(info) = read_server_info() {
        if info.model == model_name && info.lang == lang && probe(info.port) {
            SERVER_PORT.store(info.port, Ordering::Relaxed);
            touch_server();
            return Ok(info.port);
        }
        // 存在但不健康或档位不符 → 清理
        kill_pid(info.pid);
    }"""),

("""    let log_file = crate::APP_HANDLE.get().and_then(|app| {
        let d = app.path().app_config_dir().ok()?;
        let _ = std::fs::create_dir_all(&d);
        std::fs::File::create(d.join("whisper-server.log")).ok()
    });""",
"""    let log_file = std::fs::File::create(
        crate::config::data_dir().join("whisper-server.log"),
    )
    .ok();"""),

("""            if let Some(app) = crate::APP_HANDLE.get() {
                write_server_info(app, &ServerInfo { port, pid, model: model_name, lang });
            }""",
"""            write_server_info(&ServerInfo { port, pid, model: model_name, lang });"""),

("""            let tail = crate::APP_HANDLE
                .get()
                .and_then(|app| {
                    let p = app.path().app_config_dir().ok()?.join("whisper-server.log");
                    std::fs::read_to_string(p).ok()
                })
                .map(|s| {""",
"""            let tail = std::fs::read_to_string(
                crate::config::data_dir().join("whisper-server.log"),
            )
            .ok()
            .map(|s| {"""),

("""    if let Some(app) = crate::APP_HANDLE.get() {
        if let Some(info) = read_server_info(app) {
            kill_pid(info.pid);
            if let Some(path) = server_info_path(app) {
                let _ = std::fs::remove_file(path);
            }
        }
    }""",
"""    if let Some(info) = read_server_info() {
        kill_pid(info.pid);
        if let Some(path) = server_info_path() {
            let _ = std::fs::remove_file(path);
        }
    }"""),

("""    let cfg = match crate::APP_HANDLE.get() {
        Some(app) => crate::config::load(app),
        None => return,
    };
    let _ = ensure_server(&cfg).await;""",
"""    let cfg = crate::config::load();
    let _ = ensure_server(&cfg).await;"""),

("""    let cfg = {
        match crate::APP_HANDLE.get() {
            Some(app) => crate::config::load(app),
            None => crate::config::Config::default(),
        }
    };""",
"""    let cfg = crate::config::load();"""),

("""/// 预热检查 + 基准（供诊断：invoke('probe_env')）
#[tauri::command]
pub fn probe_env() -> Result<String, String> {""",
"""/// 预热检查 + 基准（诊断用）
pub fn probe_env() -> Result<String, String> {"""),
]

# ============ tools.rs ============
TOOLS = [
("""async fn show_dialog(v: &Value) -> ToolResult {
    use tauri::Manager;
    let p = dialog_params(v)?;
    // 父窗口：主窗可见时弹窗贴合其上（HWND 以 isize 跨线程传递）
    let parent = crate::APP_HANDLE
        .get()
        .and_then(|app| app.get_webview_window("main"))
        .filter(|w| w.is_visible().unwrap_or(false))
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
        .unwrap_or(0);""",
"""async fn show_dialog(v: &Value) -> ToolResult {
    let p = dialog_params(v)?;
    // 父窗口：主窗可见时弹窗贴合其上（HWND 以 isize 跨线程传递，UI 线程注册到 WinCtl）
    let ctl = crate::win_ctl();
    let hwnd = ctl.hwnd();
    let parent = if ctl.visible() && hwnd != 0 { hwnd } else { 0 };"""),
]

apply("llm.rs", LLM)
apply("voice.rs", VOICE)
apply("tools.rs", TOOLS)
print("ALL PATCHES APPLIED")
