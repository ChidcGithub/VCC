use serde::de::DeserializeOwned;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

// 锁覆盖完整读-改-写事务，而不只是最后的 rename；不跨 await 持有。
static STORAGE_LOCK: Mutex<()> = Mutex::new(());
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn transaction() -> Result<MutexGuard<'static, ()>, String> {
    STORAGE_LOCK
        .lock()
        .map_err(|_| "存储事务锁已中毒，请重启后重试".into())
}

pub(crate) fn unique_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:x}-{nanos:x}-{:x}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("读取 {} 失败：{e}", path.display())),
    }
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    read_bytes(path)?
        .map(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|e| format!("{} 数据损坏，原文件未修改：{e}", path.display()))
        })
        .transpose()
}

struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        // 只清理本次创建的临时文件，绝不删除目标或旧版历史。
        let _ = fs::remove_file(&self.0);
    }
}

pub(crate) fn atomic_write(path: &Path, data: &str) -> Result<(), String> {
    atomic_write_with(path, data, |from, to| fs::rename(from, to))
}

fn atomic_write_with(
    path: &Path,
    data: &str,
    replace: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| format!("创建数据目录失败：{e}"))?;
    let name = path.file_name().ok_or("存储目标必须是文件")?;
    let (mut file, temp) = loop {
        let mut tmp_name = name.to_os_string();
        tmp_name.push(format!(".{}.tmp", unique_id()));
        let tmp = parent.join(tmp_name);
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (file, TempFile(tmp)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("创建临时文件失败：{e}")),
        }
    };
    let written = file
        .write_all(data.as_bytes())
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all());
    // Windows 替换前必须关闭临时文件句柄；失败清理同样须在关闭之后。
    drop(file);
    written.map_err(|e| format!("写入 {} 失败，旧文件未修改：{e}", path.display()))?;
    // std::fs::rename 在 Windows 使用带替换语义的 MoveFileExW。
    // 临时文件与目标同目录，不做 remove + rename，也不降级成非原子复制。
    replace(&temp.0, path).map_err(|e| format!("替换 {} 失败，旧文件未修改：{e}", path.display()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct TestDir(pub PathBuf);

    impl TestDir {
        pub(crate) fn new() -> Self {
            let path = std::env::temp_dir().join(format!("vcc-storage-test-{}", unique_id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn atomic_replace_failure_preserves_old_file() {
        let dir = TestDir::new();
        let path = dir.0.join("data.json");
        fs::write(&path, "old").unwrap();
        let result = atomic_write_with(&path, "new", |temp, _| {
            assert_eq!(fs::read_to_string(temp).unwrap(), "new");
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "注入替换失败",
            ))
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = TestDir::new();
        let path = dir.0.join("data.json");
        atomic_write(&path, "old").unwrap();
        atomic_write(&path, "new").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "new");
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_atomic_writes_use_unique_temporary_files() {
        let dir = TestDir::new();
        let path = dir.0.join("data.json");
        std::thread::scope(|scope| {
            for n in 0..16 {
                let path = &path;
                scope.spawn(move || atomic_write(path, &n.to_string()).unwrap());
            }
        });
        assert!(fs::read_to_string(path).unwrap().parse::<u8>().unwrap() < 16);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
}
