/* ---------- 对话历史持久化 + AI 长期记忆 ---------- */
/* sessions.json: 多会话存储（id/标题/时间/消息），对标 chat.deepseek.com 的会话列表
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

fn sessions_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("sessions.json"))
}

fn legacy_history_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("history.json"))
}

fn memory_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("memory.json"))
}

/* ---------- 多会话存储 ---------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
}

/// 会话列表项（轻量，不含消息体）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionsFile {
    #[serde(default)]
    current_id: String,
    #[serde(default)]
    sessions: Vec<Session>,
}

fn gen_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("s-{nanos:032x}")
}

/// 读取会话文件；首次运行时把旧版单文件 history.json 迁移成一个会话
fn load_sessions_file(app: &AppHandle) -> SessionsFile {
    if let Ok(path) = sessions_path(app) {
        if let Ok(s) = fs::read_to_string(&path) {
            match serde_json::from_str::<SessionsFile>(&s) {
                Ok(f) => return f,
                Err(_) => {
                    // 损坏文件保留现场（.bad）供人工恢复，绝不用空文件覆盖
                    let _ = fs::rename(&path, path.with_extension("json.bad"));
                }
            }
        }
    }
    // 迁移：旧 history.json → 单个会话（标题「历史对话」）
    let legacy = if let Ok(p) = legacy_history_path(app) {
        fs::read_to_string(p)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<ChatMessage>>(&s).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut file = SessionsFile::default();
    if !legacy.is_empty() {
        let id = gen_session_id();
        file.current_id = id.clone();
        file.sessions.push(Session {
            id,
            title: "历史对话".into(),
            created_at: crate::llm::chrono_now_cn(),
            updated_at: crate::llm::chrono_now_cn(),
            messages: legacy,
        });
    }
    persist_sessions_file(app, &file);
    let _ = fs::remove_file(legacy_history_path(app).unwrap_or_default()); // 迁移完成即清理
    file
}

/// 原子写：临时文件 + rename 替换（进程中途被杀/断电不留半截 JSON）
fn atomic_write(path: &std::path::Path, data: &str) {
    // tmp 名带纳秒+pid：未串行化的并发写对（如后台总结 vs 手动清记忆）不互踩临时文件
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = path.with_extension(format!("json.{:x}.{}.tmp", nanos, std::process::id()));
    if fs::write(&tmp, data).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

fn persist_sessions_file(app: &AppHandle, file: &SessionsFile) {
    if let Ok(path) = sessions_path(app) {
        if let Ok(json) = serde_json::to_string_pretty(file) {
            atomic_write(&path, &json);
        }
    }
}

/// 把运行时历史写回当前会话（agent 轮次结束落盘 + 切换会话前保存）
pub fn save_history(app: &AppHandle, hist: &[ChatMessage]) {
    let mut file = load_sessions_file(app);
    if file.current_id.is_empty() {
        if hist.is_empty() {
            return;
        }
        let id = gen_session_id();
        file.current_id = id.clone();
        file.sessions.push(Session {
            id,
            title: String::new(),
            created_at: crate::llm::chrono_now_cn(),
            updated_at: crate::llm::chrono_now_cn(),
            messages: Vec::new(),
        });
    }
    if let Some(s) = file.sessions.iter_mut().find(|s| s.id == file.current_id) {
        s.messages = hist.to_vec();
        s.updated_at = crate::llm::chrono_now_cn();
        // 自动标题：未命名会话用第一条用户消息截 24 字
        if s.title.trim().is_empty() {
            if let Some(first) = hist.iter().find(|m| m.role == "user") {
                if let Some(c) = &first.content {
                    let t: String = c.trim().chars().take(24).collect();
                    if !t.is_empty() {
                        s.title = t;
                    }
                }
            }
        }
    }
    persist_sessions_file(app, &file);
}

/// 启动恢复：返回当前会话的运行时历史
pub fn load_history(app: &AppHandle) -> (String, Vec<ChatMessage>) {
    let mut file = load_sessions_file(app);
    if file.sessions.is_empty() {
        persist_sessions_file(app, &file);
        return (String::new(), Vec::new());
    }
    if file.current_id.is_empty() || !file.sessions.iter().any(|s| s.id == file.current_id) {
        // 指向失效 → 取最近更新的会话
        if let Some(latest) = file
            .sessions
            .iter()
            .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
        {
            file.current_id = latest.id.clone();
            persist_sessions_file(app, &file);
        }
    }
    let msgs = file
        .sessions
        .iter()
        .find(|s| s.id == file.current_id)
        .map(|s| s.messages.clone())
        .unwrap_or_default();
    (file.current_id, msgs)
}

/// 会话列表（按更新时间倒序）
pub fn list_sessions(app: &AppHandle) -> Vec<SessionMeta> {
    let mut metas: Vec<SessionMeta> = load_sessions_file(app)
        .sessions
        .into_iter()
        .map(|s| SessionMeta {
            id: s.id,
            title: if s.title.trim().is_empty() {
                "新对话".into()
            } else {
                s.title
            },
            updated_at: s.updated_at,
        })
        .collect();
    metas.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    metas
}

/// 新建会话并切换：返回新会话 id（调用方负责先把旧历史写回）
pub fn new_session(app: &AppHandle) -> String {
    let mut file = load_sessions_file(app);
    let id = gen_session_id();
    file.sessions.push(Session {
        id: id.clone(),
        title: String::new(),
        created_at: crate::llm::chrono_now_cn(),
        updated_at: crate::llm::chrono_now_cn(),
        messages: Vec::new(),
    });
    file.current_id = id.clone();
    persist_sessions_file(app, &file);
    id
}

/// 切换会话：返回该会话消息（调用方先写回旧会话）
pub fn switch_session(app: &AppHandle, id: &str) -> Result<Vec<ChatMessage>, String> {
    let mut file = load_sessions_file(app);
    let Some(s) = file.sessions.iter().find(|s| s.id == id) else {
        return Err("会话不存在".into());
    };
    let msgs = s.messages.clone();
    file.current_id = id.to_string();
    persist_sessions_file(app, &file);
    Ok(msgs)
}

/// 删除会话：若删的是当前会话则自动切到最近更新的；返回新的 current_id（可为空 = 无会话）
pub fn delete_session(app: &AppHandle, id: &str) -> String {
    let mut file = load_sessions_file(app);
    file.sessions.retain(|s| s.id != id);
    if file.current_id == id {
        file.current_id = file
            .sessions
            .iter()
            .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
            .map(|s| s.id.clone())
            .unwrap_or_default();
    }
    persist_sessions_file(app, &file);
    file.current_id
}

/// 重命名会话
pub fn rename_session(app: &AppHandle, id: &str, title: &str) {
    let mut file = load_sessions_file(app);
    if let Some(s) = file.sessions.iter_mut().find(|s| s.id == id) {
        let t = title.trim();
        s.title = if t.is_empty() { "新对话".into() } else { t.chars().take(40).collect() };
    }
    persist_sessions_file(app, &file);
}

/// 清空全部会话（设置里「清除所有对话」；记忆不受影响）
pub fn clear_all_sessions(app: &AppHandle) {
    persist_sessions_file(app, &SessionsFile::default());
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
        atomic_write(&path, &json);
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

static SUMMARIZING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 后台总结入口：永不 panic、失败静默——记忆是增强功能，绝不干扰主对话流程。
pub async fn summarize_into_memory(app: AppHandle, msgs: Vec<ChatMessage>) {
    // 单飞：总结耗时可达分钟级，并发触发时都读同一份旧记忆再整文件覆盖，
    // 后写者会抹掉先写者的成果——后到的直接跳过
    if SUMMARIZING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let _ = summarize_inner(&app, &msgs).await; // 静默：网络故障/无 Key 时不弹错
    SUMMARIZING.store(false, std::sync::atomic::Ordering::SeqCst);
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
