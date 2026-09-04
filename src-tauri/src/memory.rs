/* ---------- 对话历史持久化 + AI 长期记忆 ---------- */
/* history.json: 每轮 agent 结束后落盘，启动时恢复（跨重启的对话流）
   memory.json:  AI 自动总结的长期记忆（用户偏好/设备环境/常用指令），注入 system prompt */

use crate::llm::ChatMessage;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

/* ---------- 路径 ---------- */

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|_| "无法定位数据目录".to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn history_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("history.json"))
}

fn memory_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("memory.json"))
}

/* ---------- 对话历史 ---------- */

pub fn save_history(app: &AppHandle, hist: &[ChatMessage]) {
    let Ok(path) = history_path(app) else { return };
    if let Ok(json) = serde_json::to_string(hist) {
        let _ = fs::write(path, json);
    }
}

pub fn load_history(app: &AppHandle) -> Vec<ChatMessage> {
    let Ok(path) = history_path(app) else {
        return Vec::new();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn clear_history(app: &AppHandle) {
    if let Ok(path) = history_path(app) {
        let _ = fs::remove_file(path);
    }
}

/* ---------- AI 长期记忆 ---------- */

#[derive(Debug, Serialize, Deserialize)]
struct MemoryFile {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    updated_at: String,
}

pub fn load_memory(app: &AppHandle) -> String {
    let Ok(path) = memory_path(app) else {
        return String::new();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<MemoryFile>(&s).ok())
        .map(|m| m.summary)
        .unwrap_or_default()
}

pub fn save_memory(app: &AppHandle, summary: &str) {
    let Ok(path) = memory_path(app) else { return };
    let file = MemoryFile {
        summary: summary.to_string(),
        updated_at: crate::llm::chrono_now_cn(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&file) {
        let _ = fs::write(path, json);
    }
}

/* ---------- AI 记忆总结（后台异步，失败静默） ---------- */

/// 把消息列表压成紧凑对话稿，供总结用。每条截断 200 字，总量保尾 6000 字。
fn render_transcript(msgs: &[ChatMessage]) -> String {
    let mut out = String::new();
    for m in msgs {
        match m.role.as_str() {
            "user" => {
                if let Some(c) = m.content.as_deref() {
                    let line: String = format!("用户: {c}").chars().take(200).collect();
                    out.push_str(&line);
                    out.push('\n');
                }
            }
            "assistant" => {
                if let Some(c) = m.content.as_deref() {
                    if !c.trim().is_empty() {
                        let line: String = format!("AI: {c}").chars().take(200).collect();
                        out.push_str(&line);
                        out.push('\n');
                    }
                }
                if let Some(tcs) = &m.tool_calls {
                    for tc in tcs {
                        out.push_str(&format!("[调用工具] {}\n", tc.function.name));
                    }
                }
            }
            "tool" => {
                if let Some(c) = m.content.as_deref() {
                    let line: String = format!("[工具结果] {c}").chars().take(160).collect();
                    out.push_str(&line);
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
    let chars: Vec<char> = out.chars().collect();
    if chars.len() > 6000 {
        chars[chars.len() - 6000..].iter().collect()
    } else {
        out
    }
}

/// 后台总结入口：永不 panic、失败静默——记忆是增强功能，绝不干扰主对话流程。
pub async fn summarize_into_memory(app: AppHandle, msgs: Vec<ChatMessage>) {
    if let Err(_e) = summarize_inner(&app, &msgs).await {
        // 静默：网络故障/无 Key 时不弹错、不打断
    }
}

async fn summarize_inner(app: &AppHandle, msgs: &[ChatMessage]) -> Result<(), String> {
    // 没有用户实质发言就不总结（纯工具轮/空历史）
    if !msgs.iter().any(|m| m.role == "user") {
        return Ok(());
    }
    let cfg = crate::config::load(app);
    if cfg.api_key.is_empty() {
        return Ok(());
    }

    let old = load_memory(app);
    let transcript = render_transcript(msgs);
    if transcript.trim().is_empty() {
        return Ok(());
    }

    let prompt = format!(
        "你是 VCC 课堂助手的记忆模块。根据【旧记忆】和【对话记录】，输出更新后的用户长期记忆。\n\
要求：\n\
- 只保留对后续交互长期有价值的信息：用户身份与习惯、这台电脑的设备环境、常用指令与偏好、反复出现的任务模式。\n\
- 不要记录一次性任务细节、命令输出、寒暄。\n\
- 控制在 200 字以内，分条简洁陈述，直接输出内容本身，不要任何前缀、标题或解释。\n\
- 若对话没有新的可记忆内容，原样输出旧记忆，不要改写。\n\n\
【旧记忆】\n{}\n\n【对话记录】\n{}",
        if old.is_empty() { "（空）" } else { old.as_str() },
        transcript
    );

    let messages = vec![ChatMessage::text("user", &prompt)];
    let summary = crate::llm::chat_completion_simple(&cfg, &messages).await?;
    let summary = summary.trim().trim_matches('"').trim().to_string();
    if summary.is_empty() {
        return Ok(());
    }

    save_memory(app, &summary);
    let _ = app.emit(
        "vcc://memory-updated",
        serde_json::json!({ "summary": summary }),
    );
    Ok(())
}
