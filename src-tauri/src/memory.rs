/* ---------- 对话历史持久化 + AI 长期记忆 ---------- */

use crate::config::{data_dir, storage};
use crate::llm::ChatMessage;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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
    sessions: Vec<Session>,
}

impl SessionsFile {
    fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        for session in &self.sessions {
            if session.id.trim().is_empty() || !ids.insert(&session.id) {
                return Err("会话文件包含空白或重复 id，原文件未修改".into());
            }
        }
        Ok(())
    }

    fn latest_id(&self) -> String {
        self.sessions
            .iter()
            .max_by(|a, b| {
                timestamp_key(&a.updated_at)
                    .cmp(&timestamp_key(&b.updated_at))
                    .then_with(|| a.id.cmp(&b.id))
            })
            .map(|s| s.id.clone())
            .unwrap_or_default()
    }

    fn repair_current(&mut self) {
        if !self.sessions.iter().any(|s| s.id == self.current_id) {
            self.current_id = self.latest_id();
        }
    }
}

/// 显式注入路径，测试不读取或修改 APPDATA / VCC_DATA_DIR。
struct Store {
    dir: PathBuf,
}

impl Store {
    fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    // 以下 unlocked 方法只能在持有 storage::transaction 时调用。
    fn read_sessions_unlocked(&self) -> Result<SessionsFile, String> {
        self.read_sessions_with(|file| self.persist_sessions_unlocked(file))
    }

    fn read_sessions_with(
        &self,
        persist: impl FnOnce(&SessionsFile) -> Result<(), String>,
    ) -> Result<SessionsFile, String> {
        if let Some(mut file) = storage::read_json::<SessionsFile>(&self.dir.join("sessions.json"))?
        {
            file.validate()?;
            // 失效 current_id 只在内存中修复，纯读取不改磁盘。
            file.repair_current();
            return Ok(file);
        }
        let mut file = SessionsFile::default();
        if let Some(legacy) =
            storage::read_json::<Vec<ChatMessage>>(&self.dir.join("history.json"))?
        {
            if !legacy.is_empty() {
                let mut session = empty_session();
                session.title = "历史对话".into();
                session.messages = legacy;
                file.current_id = session.id.clone();
                file.sessions.push(session);
            }
            // 包括空历史也写迁移标记；失败向上传播，旧 history.json 永远保留。
            persist(&file)?;
        }
        Ok(file)
    }

    fn persist_sessions_unlocked(&self, file: &SessionsFile) -> Result<(), String> {
        file.validate()?;
        let json = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
        storage::atomic_write(&self.dir.join("sessions.json"), &json)
    }

    fn update_sessions<T>(
        &self,
        update: impl FnOnce(&mut SessionsFile) -> Result<T, String>,
    ) -> Result<T, String> {
        let _transaction = storage::transaction()?;
        let mut file = self.read_sessions_unlocked()?;
        let result = update(&mut file)?;
        self.persist_sessions_unlocked(&file)?;
        Ok(result)
    }

    fn load_history(&self) -> Result<(String, Vec<ChatMessage>), String> {
        let _transaction = storage::transaction()?;
        let file = self.read_sessions_unlocked()?;
        let messages = file
            .sessions
            .iter()
            .find(|s| s.id == file.current_id)
            .map(|s| s.messages.clone())
            .unwrap_or_default();
        Ok((file.current_id, messages))
    }

    fn list_sessions(&self) -> Result<Vec<SessionMeta>, String> {
        let _transaction = storage::transaction()?;
        let mut metas: Vec<_> = self
            .read_sessions_unlocked()?
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
        metas.sort_by(|a, b| {
            timestamp_key(&b.updated_at)
                .cmp(&timestamp_key(&a.updated_at))
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(metas)
    }

    fn save_history(&self, hist: &[ChatMessage]) -> Result<(), String> {
        self.update_sessions(|file| {
            if file.current_id.is_empty() {
                if hist.is_empty() {
                    return Ok(());
                }
                let session = empty_session();
                file.current_id = session.id.clone();
                file.sessions.push(session);
            }
            let id = file.current_id.clone();
            update_history(file, &id, hist)
        })
    }

    fn save_history_for(&self, session_id: &str, hist: &[ChatMessage]) -> Result<(), String> {
        self.update_sessions(|file| update_history(file, session_id, hist))
    }

    fn new_session(&self) -> Result<String, String> {
        self.update_sessions(|file| {
            let session = empty_session();
            file.current_id = session.id.clone();
            file.sessions.push(session);
            Ok(file.current_id.clone())
        })
    }

    fn switch_session(&self, id: &str) -> Result<Vec<ChatMessage>, String> {
        self.update_sessions(|file| {
            let session = file
                .sessions
                .iter()
                .find(|s| s.id == id)
                .ok_or("会话不存在")?;
            let messages = session.messages.clone();
            file.current_id = id.to_string();
            Ok(messages)
        })
    }

    fn delete_session(&self, id: &str) -> Result<String, String> {
        self.update_sessions(|file| {
            if !file.sessions.iter().any(|s| s.id == id) {
                return Err("会话不存在".into());
            }
            file.sessions.retain(|s| s.id != id);
            file.repair_current();
            Ok(file.current_id.clone())
        })
    }

    fn rename_session(&self, id: &str, title: &str) -> Result<(), String> {
        self.update_sessions(|file| {
            let session = file
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("会话不存在")?;
            let title = title.trim();
            session.title = if title.is_empty() {
                "新对话".into()
            } else {
                title.chars().take(40).collect()
            };
            Ok(())
        })
    }

    fn clear_all_sessions(&self) -> Result<(), String> {
        // 必须先读并校验旧文件，不能用“清空”绕过损坏保护。
        self.update_sessions(|file| {
            *file = SessionsFile::default();
            Ok(())
        })
    }

    fn memory_snapshot_unlocked(&self) -> Result<MemorySnapshot, String> {
        let path = self.dir.join("memory.json");
        let bytes = storage::read_bytes(&path)?;
        let summary = match bytes.as_deref() {
            Some(bytes) => {
                serde_json::from_slice::<MemoryFile>(bytes)
                    .map_err(|e| format!("{} 数据损坏，原文件未修改：{e}", path.display()))?
                    .summary
            }
            None => String::new(),
        };
        Ok(MemorySnapshot { bytes, summary })
    }

    fn memory_snapshot(&self) -> Result<MemorySnapshot, String> {
        let _transaction = storage::transaction()?;
        self.memory_snapshot_unlocked()
    }

    fn persist_memory_unlocked(&self, summary: &str) -> Result<(), String> {
        let file = MemoryFile {
            summary: summary.to_string(),
            updated_at: crate::llm::chrono_now_cn(),
            // 每次成功保存都会换版本，即使内容与分钟级时间戳没变（防 ABA）。
            revision: storage::unique_id(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        storage::atomic_write(&self.dir.join("memory.json"), &json)
    }

    fn save_memory(&self, summary: &str) -> Result<(), String> {
        let _transaction = storage::transaction()?;
        self.memory_snapshot_unlocked()?;
        self.persist_memory_unlocked(summary)
    }

    fn save_summary_if_unchanged(
        &self,
        snapshot: &MemorySnapshot,
        summary: &str,
    ) -> Result<bool, String> {
        let _transaction = storage::transaction()?;
        if self.memory_snapshot_unlocked()?.bytes != snapshot.bytes {
            return Ok(false);
        }
        self.persist_memory_unlocked(summary)?;
        Ok(true)
    }
}

fn empty_session() -> Session {
    let now = crate::llm::chrono_now_cn();
    Session {
        id: format!("s-{}", storage::unique_id()),
        title: String::new(),
        created_at: now.clone(),
        updated_at: now,
        messages: Vec::new(),
    }
}

fn update_history(file: &mut SessionsFile, id: &str, hist: &[ChatMessage]) -> Result<(), String> {
    let session = file
        .sessions
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or("会话不存在或已删除，未保存历史")?;
    session.messages = hist.to_vec();
    session.updated_at = crate::llm::chrono_now_cn();
    if session.title.trim().is_empty() {
        if let Some(content) = hist
            .iter()
            .find(|m| m.role == "user")
            .and_then(|m| m.content.as_deref())
        {
            session.title = content.trim().chars().take(24).collect();
        }
    }
    Ok(())
}

pub fn save_history(hist: &[ChatMessage]) -> Result<(), String> {
    Store::new(data_dir()).save_history(hist)
}

/// 后台轮次使用捕获的 id 保存；不跟随 current_id，也不复活已删除的会话。
pub fn save_history_for(session_id: &str, hist: &[ChatMessage]) -> Result<(), String> {
    Store::new(data_dir()).save_history_for(session_id, hist)
}

pub fn try_load_history() -> Result<(String, Vec<ChatMessage>), String> {
    Store::new(data_dir()).load_history()
}

pub fn load_history() -> (String, Vec<ChatMessage>) {
    try_load_history().unwrap_or_else(|e| {
        eprintln!("vcc: 读取会话失败：{e}");
        (String::new(), Vec::new())
    })
}

pub fn try_list_sessions() -> Result<Vec<SessionMeta>, String> {
    Store::new(data_dir()).list_sessions()
}

pub fn list_sessions() -> Vec<SessionMeta> {
    try_list_sessions().unwrap_or_else(|e| {
        eprintln!("vcc: 读取会话列表失败：{e}");
        Vec::new()
    })
}

pub fn new_session() -> Result<String, String> {
    Store::new(data_dir()).new_session()
}

pub fn switch_session(id: &str) -> Result<Vec<ChatMessage>, String> {
    Store::new(data_dir()).switch_session(id)
}

pub fn delete_session(id: &str) -> Result<String, String> {
    Store::new(data_dir()).delete_session(id)
}

pub fn rename_session(id: &str, title: &str) -> Result<(), String> {
    Store::new(data_dir()).rename_session(id, title)
}

pub fn clear_all_sessions() -> Result<(), String> {
    Store::new(data_dir()).clear_all_sessions()
}

/* ---------- 旧中文时间串按真实公历排序/分组 ---------- */

fn timestamp_key(value: &str) -> Option<(i64, u32)> {
    let (year, rest) = value.trim().split_once('年')?;
    let (month, rest) = rest.split_once('月')?;
    let (day, time) = rest.split_once('日')?;
    let (year, month, day) = (
        year.parse::<i64>().ok()?,
        month.parse::<usize>().ok()?,
        day.parse::<u32>().ok()?,
    );
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day == 0 || day > months[month - 1] {
        return None;
    }
    let time = time.trim();
    let seconds = if time.is_empty() {
        0
    } else {
        let parts = time
            .split(':')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        if !(2..=3).contains(&parts.len())
            || parts[0] > 23
            || parts[1] > 59
            || parts.get(2).copied().unwrap_or(0) > 59
        {
            return None;
        }
        parts[0] * 3600 + parts[1] * 60 + parts.get(2).copied().unwrap_or(0)
    };
    let previous = year - 1;
    let days = previous * 365 + previous / 4 - previous / 100
        + previous / 400
        + months[..month - 1].iter().sum::<u32>() as i64
        + day as i64
        - 1;
    Some((days, seconds))
}

/// 分组边界为自然日：今天、昨天、2..=6 天、至少 7 天；未来日期归今天。
pub fn session_group_at(updated: &str, now: &str) -> &'static str {
    match (timestamp_key(updated), timestamp_key(now)) {
        (Some((updated, _)), Some((today, _))) => match today - updated {
            ..=0 => "今天",
            1 => "昨天",
            2..=6 => "7 天内",
            _ => "更早",
        },
        _ => "更早",
    }
}

/* ---------- AI 长期记忆 ---------- */

#[derive(Debug, Serialize, Deserialize)]
struct MemoryFile {
    summary: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    revision: String,
}

struct MemorySnapshot {
    bytes: Option<Vec<u8>>,
    summary: String,
}

pub fn try_load_memory() -> Result<String, String> {
    Store::new(data_dir()).memory_snapshot().map(|s| s.summary)
}

pub fn load_memory() -> String {
    try_load_memory().unwrap_or_else(|e| {
        eprintln!("vcc: 读取长期记忆失败：{e}");
        String::new()
    })
}

pub fn save_memory(summary: &str) -> Result<(), String> {
    Store::new(data_dir()).save_memory(summary)
}

/// 每条截断 200 字，总量保尾 6000 字。
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

static SUMMARIZING: AtomicBool = AtomicBool::new(false);

struct SummaryGuard<'a>(&'a AtomicBool);

impl<'a> SummaryGuard<'a> {
    fn acquire(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self(flag))
    }
}

impl Drop for SummaryGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

async fn run_summary(
    flag: &AtomicBool,
    timeout: Duration,
    work: impl std::future::Future<Output = Result<(), String>>,
) -> Result<(), String> {
    let Some(_guard) = SummaryGuard::acquire(flag) else {
        return Ok(());
    };
    tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| "长期记忆总结超时".to_string())?
}

/// 取消、超时、提前返回或 unwind 都由 RAII 释放单飞标记；不持存储锁等待网络。
pub async fn summarize_into_memory(msgs: Vec<ChatMessage>) {
    if let Err(e) = run_summary(
        &SUMMARIZING,
        Duration::from_secs(120),
        summarize_inner(&msgs),
    )
    .await
    {
        eprintln!("vcc: 长期记忆总结未完成：{e}");
    }
}

async fn summarize_inner(msgs: &[ChatMessage]) -> Result<(), String> {
    if !msgs.iter().any(|m| m.role == "user") {
        return Ok(());
    }
    let cfg = crate::config::try_load()?;
    if cfg.api_key.is_empty() {
        return Ok(());
    }
    // 固定本次总结的数据目录和读取版本，写入时再次校验。
    let store = Store::new(data_dir());
    let snapshot = store.memory_snapshot()?;
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
        if snapshot.summary.is_empty() { "（空）" } else { snapshot.summary.as_str() }, transcript
    );
    let messages = vec![ChatMessage::text("user", &prompt)];
    let summary = crate::llm::chat_completion_simple(&cfg, &messages).await?;
    let summary = summary.trim().trim_matches('"').trim();
    if !summary.is_empty() && !store.save_summary_if_unchanged(&snapshot, summary)? {
        eprintln!("vcc: 长期记忆已被修改，跳过过期总结");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Barrier};
    use storage::tests::TestDir;

    fn fixture() -> (TestDir, Store) {
        let dir = TestDir::new();
        let store = Store::new(dir.0.clone());
        (dir, store)
    }

    #[test]
    fn concurrent_session_updates_are_not_lost() {
        let (_dir, store) = fixture();
        let barrier = Barrier::new(16);
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for n in 0..16 {
                let (store, barrier) = (&store, &barrier);
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    let id = store.new_session().unwrap();
                    store.rename_session(&id, &format!("标题{n}")).unwrap();
                    store
                        .save_history_for(&id, &[ChatMessage::text("user", &n.to_string())])
                        .unwrap();
                    (id, n)
                }));
            }
            for handle in handles {
                let (id, n) = handle.join().unwrap();
                let msgs = store.switch_session(&id).unwrap();
                assert_eq!(msgs[0].content.as_deref(), Some(n.to_string().as_str()));
            }
        });
        assert_eq!(store.list_sessions().unwrap().len(), 16);
        assert!(store
            .list_sessions()
            .unwrap()
            .iter()
            .all(|s| s.title.starts_with("标题")));
    }

    #[test]
    fn bound_history_does_not_follow_current_or_revive_deleted_session() {
        let (dir, store) = fixture();
        let first = store.new_session().unwrap();
        let second = store.new_session().unwrap();
        store
            .save_history_for(&first, &[ChatMessage::text("user", "第一条")])
            .unwrap();
        let (current, msgs) = store.load_history().unwrap();
        assert_eq!(current, second);
        assert!(msgs.is_empty());
        assert_eq!(
            store.switch_session(&first).unwrap()[0].content.as_deref(),
            Some("第一条")
        );
        store.delete_session(&first).unwrap();
        let before = fs::read(dir.0.join("sessions.json")).unwrap();
        assert!(store.save_history_for(&first, &[]).is_err());
        assert_eq!(fs::read(dir.0.join("sessions.json")).unwrap(), before);
        assert_eq!(store.load_history().unwrap().0, second);
    }

    #[test]
    fn corrupt_sessions_block_all_writes_without_moving_files() {
        let (dir, store) = fixture();
        let path = dir.0.join("sessions.json");
        fs::write(dir.0.join("history.json"), "[]").unwrap();
        for contents in [
            "{bad",
            "{}",
            r#"{"sessions":[{"id":"x","title":"a"},{"id":"x","title":"b"}]}"#,
        ] {
            fs::write(&path, contents).unwrap();
            assert!(store.load_history().is_err());
            assert!(store.list_sessions().is_err());
            assert!(store.save_history(&[]).is_err());
            assert!(store.save_history_for("x", &[]).is_err());
            assert!(store.new_session().is_err());
            assert!(store.switch_session("x").is_err());
            assert!(store.delete_session("x").is_err());
            assert!(store.rename_session("x", "标题").is_err());
            assert!(store.clear_all_sessions().is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
            assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
        }
    }

    #[test]
    fn migration_preserves_legacy_and_clear_does_not_resurrect_it() {
        let (dir, store) = fixture();
        let legacy = serde_json::to_vec(&vec![ChatMessage::text("user", "旧历史")]).unwrap();
        fs::write(dir.0.join("history.json"), &legacy).unwrap();
        let (id, msgs) = store.load_history().unwrap();
        assert!(!id.is_empty());
        assert_eq!(msgs[0].content.as_deref(), Some("旧历史"));
        assert_eq!(fs::read(dir.0.join("history.json")).unwrap(), legacy);
        assert_eq!(store.load_history().unwrap().0, id);
        store.clear_all_sessions().unwrap();
        assert!(store.load_history().unwrap().1.is_empty());
        assert!(store.list_sessions().unwrap().is_empty());
        assert_eq!(fs::read(dir.0.join("history.json")).unwrap(), legacy);
    }

    #[test]
    fn migration_write_failure_is_reported_and_preserves_legacy() {
        let (dir, store) = fixture();
        let bytes = br#"[{"role":"user","content":"old"}]"#;
        fs::write(dir.0.join("history.json"), bytes).unwrap();
        let result = {
            let _transaction = storage::transaction().unwrap();
            store.read_sessions_with(|_| Err("注入迁移写入失败".into()))
        };
        assert_eq!(result.unwrap_err(), "注入迁移写入失败");
        assert!(!dir.0.join("sessions.json").exists());
        assert_eq!(fs::read(dir.0.join("history.json")).unwrap(), bytes);
        assert_eq!(store.load_history().unwrap().1.len(), 1);
    }

    #[test]
    fn corrupt_legacy_is_not_migrated_or_overwritten() {
        let (dir, store) = fixture();
        fs::write(dir.0.join("history.json"), "broken").unwrap();
        assert!(store.load_history().is_err());
        assert!(store.new_session().is_err());
        assert!(store.clear_all_sessions().is_err());
        assert_eq!(
            fs::read_to_string(dir.0.join("history.json")).unwrap(),
            "broken"
        );
        assert!(!dir.0.join("sessions.json").exists());
    }

    #[test]
    fn missing_reads_do_not_create_files_and_io_errors_are_reported() {
        let (dir, store) = fixture();
        assert_eq!(store.load_history().unwrap().0, "");
        assert!(store.list_sessions().unwrap().is_empty());
        assert!(store.memory_snapshot().unwrap().summary.is_empty());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
        fs::create_dir(dir.0.join("sessions.json")).unwrap();
        assert!(store.load_history().is_err());
        assert!(store.new_session().is_err());
        assert!(store.switch_session("missing").is_err());
        fs::write(dir.0.join("blocked"), "not a directory").unwrap();
        assert!(Store::new(dir.0.join("blocked"))
            .save_memory("test")
            .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn switch_session_persist_failure_preserves_current_on_windows() {
        use std::os::windows::fs::OpenOptionsExt;
        let (dir, store) = fixture();
        let first = store.new_session().unwrap();
        let second = store.new_session().unwrap();
        let path = dir.0.join("sessions.json");
        let before = fs::read(&path).unwrap();
        // 允许读取但不共享删除权限，真实触发 Windows 原子替换失败。
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        assert!(store.switch_session(&first).is_err());
        assert_eq!(store.load_history().unwrap().0, second);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
        drop(held);
        store.switch_session(&first).unwrap();
        assert_eq!(store.load_history().unwrap().0, first);
    }

    #[test]
    fn chinese_dates_sort_numerically_and_select_latest() {
        let (dir, store) = fixture();
        let mut file = SessionsFile::default();
        for (id, date) in [
            ("sep", "2026年9月30日 23:59"),
            ("oct", "2026年10月1日 00:00"),
            ("early", "2026年10月9日 09:01"),
            ("latest", "2026年10月10日 08:01"),
            ("bad", "无效时间"),
        ] {
            file.sessions.push(Session {
                id: id.into(),
                updated_at: date.into(),
                ..empty_session()
            });
        }
        file.current_id = "missing".into();
        let json = serde_json::to_string(&file).unwrap();
        fs::write(dir.0.join("sessions.json"), &json).unwrap();
        assert_eq!(
            store
                .list_sessions()
                .unwrap()
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            vec!["latest", "early", "oct", "sep", "bad"]
        );
        assert_eq!(store.load_history().unwrap().0, "latest");
        assert_eq!(
            fs::read_to_string(dir.0.join("sessions.json")).unwrap(),
            json
        );
        assert_eq!(store.delete_session("latest").unwrap(), "early");
        assert!(timestamp_key("2026年10月1日 9:10") < timestamp_key("2026年10月1日 10:01"));
    }

    #[test]
    fn date_groups_handle_month_year_and_leap_boundaries() {
        for (updated, now, expected) in [
            ("2026年9月30日 23:59", "2026年10月1日 00:00", "昨天"),
            ("2025年12月31日", "2026年1月1日", "昨天"),
            ("2024年2月29日", "2024年3月1日", "昨天"),
            ("2024年2月28日", "2024年3月1日", "7 天内"),
            ("2025年2月28日", "2025年3月1日", "昨天"),
            ("1900年2月28日", "1900年3月1日", "昨天"),
            ("2000年2月28日", "2000年3月1日", "7 天内"),
            ("2026年8月31日", "2026年9月6日", "7 天内"),
            ("2026年8月31日", "2026年9月7日", "更早"),
            ("2026年9月15日 00:00", "2026年9月15日 23:59", "今天"),
            ("2026年9月16日", "2026年9月15日", "今天"),
            ("2025年2月29日", "2025年3月1日", "更早"),
            ("", "2026年9月15日", "更早"),
            ("2026年9月15日", "invalid", "更早"),
        ] {
            assert_eq!(
                session_group_at(updated, now),
                expected,
                "{updated} / {now}"
            );
        }
        for date in [
            "2026年0月1日",
            "2026年13月1日",
            "2026年4月31日",
            "2026年1月0日",
            "2026年1月1日 24:00",
            "999999999999999年1月1日",
        ] {
            assert!(timestamp_key(date).is_none(), "{date}");
        }
    }

    #[test]
    fn old_memory_is_compatible_and_corrupt_memory_is_protected() {
        let (dir, store) = fixture();
        let path = dir.0.join("memory.json");
        fs::write(&path, r#"{"summary":"old"}"#).unwrap();
        assert_eq!(store.memory_snapshot().unwrap().summary, "old");
        store.save_memory("new").unwrap();
        for bad in ["{bad", "{}", r#"{"summary":42}"#] {
            fs::write(&path, bad).unwrap();
            assert!(store.memory_snapshot().is_err());
            assert!(store.save_memory("overwrite").is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), bad);
            assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
        }
    }

    #[test]
    fn stale_summary_cannot_overwrite_manual_edits_or_aba() {
        let (_dir, store) = fixture();
        let missing = store.memory_snapshot().unwrap();
        store.save_memory("").unwrap();
        assert!(!store.save_summary_if_unchanged(&missing, "stale").unwrap());
        store.save_memory("A").unwrap();
        let snapshot = store.memory_snapshot().unwrap();
        store.save_memory("B").unwrap();
        store.save_memory("A").unwrap();
        assert!(!store.save_summary_if_unchanged(&snapshot, "stale").unwrap());
        assert_eq!(store.memory_snapshot().unwrap().summary, "A");
        let snapshot = store.memory_snapshot().unwrap();
        assert!(store.save_summary_if_unchanged(&snapshot, "fresh").unwrap());
        assert_eq!(store.memory_snapshot().unwrap().summary, "fresh");
        let snapshot = store.memory_snapshot().unwrap();
        store.save_memory("").unwrap();
        assert!(!store.save_summary_if_unchanged(&snapshot, "stale").unwrap());
        assert!(store.memory_snapshot().unwrap().summary.is_empty());
    }

    #[test]
    fn external_memory_edit_invalidates_snapshot() {
        let (dir, store) = fixture();
        store.save_memory("A").unwrap();
        let snapshot = store.memory_snapshot().unwrap();
        fs::write(dir.0.join("memory.json"), r#"{"summary":"manual"}"#).unwrap();
        assert!(!store.save_summary_if_unchanged(&snapshot, "stale").unwrap());
        fs::write(dir.0.join("memory.json"), "broken").unwrap();
        assert!(store.save_summary_if_unchanged(&snapshot, "stale").is_err());
        assert_eq!(
            fs::read_to_string(dir.0.join("memory.json")).unwrap(),
            "broken"
        );
    }

    #[tokio::test]
    async fn summary_guard_releases_on_error_timeout_and_cancel() {
        let flag = Arc::new(AtomicBool::new(false));
        assert!(
            run_summary(&flag, Duration::from_secs(1), async { Err("test".into()) })
                .await
                .is_err()
        );
        assert!(!flag.load(Ordering::Acquire));
        assert!(
            run_summary(&flag, Duration::from_millis(1), std::future::pending())
                .await
                .is_err()
        );
        assert!(!flag.load(Ordering::Acquire));
        let task_flag = flag.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            run_summary(&task_flag, Duration::from_secs(60), async {
                started.send(()).unwrap();
                std::future::pending::<Result<(), String>>().await
            })
            .await
        });
        ready.await.unwrap();
        assert!(flag.load(Ordering::Acquire));
        assert!(SummaryGuard::acquire(&flag).is_none());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!flag.load(Ordering::Acquire));
        assert!(run_summary(&flag, Duration::from_secs(1), async { Ok(()) })
            .await
            .is_ok());
    }

    #[test]
    fn summary_guard_releases_on_unwind() {
        let flag = AtomicBool::new(false);
        let result = std::panic::catch_unwind(|| {
            let _guard = SummaryGuard::acquire(&flag).unwrap();
            panic!("注入任务 panic");
        });
        assert!(result.is_err());
        assert!(!flag.load(Ordering::Acquire));
    }
}
