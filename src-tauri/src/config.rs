use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[path = "storage.rs"]
pub(crate) mod storage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub always_on_top: bool,
    #[serde(default = "default_voice_model")]
    pub voice_model: String,
    #[serde(default)]
    pub voice_threads: i32,
    /// 识别语言：zh / en / auto（whisper ISO 639-1）
    #[serde(default = "default_voice_lang")]
    pub voice_lang: String,
    /// 自定义快捷指令（空态胶囊按钮，最多 8 条；空 = 用默认四条）
    #[serde(default)]
    pub custom_cmds: Vec<String>,
    /// 朗读 AI 回答（投影课堂：学生听得到回答）——egui 分支暂缓 TTS，字段保留兼容旧配置
    #[serde(default)]
    pub tts_enabled: bool,
    /// 界面主题：dark / light
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_review_transcript")]
    pub review_transcript: bool,
    #[serde(default)]
    pub reduce_motion: bool,
}

fn default_review_transcript() -> bool {
    true
}

fn default_base_url() -> String {
    "https://api.deepseek.com".into()
}
fn default_model() -> String {
    "deepseek-chat".into()
}
fn default_hotkey() -> String {
    "ctrl+shift+space".into()
}
fn default_voice_model() -> String {
    "fast".into() // q5_1 量化：学校低配设备友好
}
fn default_voice_lang() -> String {
    "zh".into()
}
fn default_theme() -> String {
    "dark".into() // VCC 暗色基因；设置面板可切浅色
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: default_base_url(),
            model: default_model(),
            hotkey: default_hotkey(),
            autostart: false,
            always_on_top: false,
            voice_model: default_voice_model(),
            voice_threads: 0,
            voice_lang: default_voice_lang(),
            custom_cmds: Vec::new(),
            tts_enabled: false,
            theme: default_theme(),
            review_transcript: true,
            reduce_motion: false,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        self.normalized().map(|_| ())
    }

    /// 只做无损格式规范化；不把非法值悄悄替换成默认值。
    pub fn normalized(&self) -> Result<Self, String> {
        if self.api_key.contains(['\r', '\n']) {
            return Err("API Key 不允许包含 CR/LF".into());
        }
        if self.base_url.chars().any(char::is_control) {
            return Err("Base URL 不允许包含控制字符".into());
        }
        let mut cfg = self.clone();
        cfg.api_key = cfg.api_key.trim().to_string();
        let base = cfg.base_url.trim();
        let (scheme, rest) = base
            .split_once("://")
            .ok_or("Base URL 必须为完整的 http(s) URL")?;
        if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
            || rest.is_empty()
            || rest.starts_with('/')
            || base.contains('\\')
            || base.chars().any(char::is_whitespace)
        {
            return Err("Base URL 必须为合法的 http(s) URL".into());
        }
        let url = reqwest::Url::parse(base).map_err(|_| "Base URL 格式无效")?;
        let host = url.host_str().ok_or("Base URL 缺少主机名")?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Base URL 不允许包含凭据、查询参数或片段".into());
        }
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .map(|ip| ip.is_loopback())
                .unwrap_or(false);
        if url.scheme() != "https" && !loopback {
            return Err("非回环地址只允许使用 HTTPS".into());
        }
        cfg.base_url = url.as_str().trim_end_matches('/').to_string();
        cfg.model = cfg.model.trim().to_string();
        if cfg.model.is_empty() || cfg.model.chars().any(char::is_control) {
            return Err("模型名称不能为空或包含控制字符".into());
        }
        cfg.voice_model = cfg.voice_model.trim().to_ascii_lowercase();
        if !matches!(cfg.voice_model.as_str(), "fast" | "quality") {
            return Err("voice_model 只允许 fast / quality".into());
        }
        cfg.voice_lang = cfg.voice_lang.trim().to_ascii_lowercase();
        if !matches!(cfg.voice_lang.as_str(), "zh" | "en" | "auto") {
            return Err("voice_lang 只允许 zh / en / auto".into());
        }
        cfg.theme = cfg.theme.trim().to_ascii_lowercase();
        if !matches!(cfg.theme.as_str(), "dark" | "light") {
            return Err("theme 只允许 dark / light".into());
        }
        if !(0..=256).contains(&cfg.voice_threads) {
            return Err("voice_threads 应为 0（自动）或 1..=256".into());
        }
        cfg.hotkey = normalized_hotkey(&cfg.hotkey)?;
        Ok(cfg)
    }
}

fn normalized_hotkey(value: &str) -> Result<String, String> {
    let lower = value.trim().to_ascii_lowercase();
    let mut tokens = Vec::new();
    let mut modifiers = std::collections::HashSet::new();
    let mut has_key = false;
    for token in lower.split('+').map(str::trim) {
        let token = match token {
            "control" | "ctl" => "ctrl",
            "super" | "meta" | "cmd" => "win",
            "escape" => "esc",
            "del" => "delete",
            other => other,
        };
        if has_key || token.is_empty() {
            return Err("热键格式应为可选修饰键 + 一个主键".into());
        }
        if matches!(token, "ctrl" | "alt" | "shift" | "win") {
            if !modifiers.insert(token) {
                return Err("热键修饰键不能重复".into());
            }
        } else {
            let valid = (token.len() == 1 && token.as_bytes()[0].is_ascii_alphanumeric())
                || matches!(
                    token,
                    "space"
                        | "up"
                        | "down"
                        | "left"
                        | "right"
                        | "tab"
                        | "enter"
                        | "esc"
                        | "backspace"
                        | "delete"
                        | "home"
                        | "end"
                        | "pageup"
                        | "pagedown"
                        | "minus"
                        | "equal"
                )
                || token
                    .strip_prefix('f')
                    .and_then(|n| n.parse::<u8>().ok())
                    .map(|n| (1..=24).contains(&n) && token == format!("f{n}"))
                    .unwrap_or(false);
            if !valid {
                return Err("热键包含不支持的主键".into());
            }
            has_key = true;
        }
        tokens.push(token);
    }
    if !has_key {
        return Err("热键缺少主键".into());
    }
    Ok(tokens.join("+"))
}

fn resolve_data_dir(
    override_dir: Option<std::ffi::OsString>,
    appdata: Option<std::ffi::OsString>,
) -> PathBuf {
    override_dir
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            appdata
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("com.chidc.vcc")
        })
}

/// VCC_DATA_DIR 直接指定数据目录；未设置时仍使用 %APPDATA%/com.chidc.vcc。
pub fn data_dir() -> PathBuf {
    let dir = resolve_data_dir(
        std::env::var_os("VCC_DATA_DIR"),
        std::env::var_os("APPDATA"),
    );
    // 保留其他调用方依赖的自动建目录行为；实际写入仍会严格报告目录错误。
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("vcc: 创建数据目录 {} 失败：{e}", dir.display());
    }
    dir
}

fn load_at(path: &Path) -> Result<Config, String> {
    storage::read_json::<Config>(path)?
        .unwrap_or_default()
        .normalized()
}

pub fn try_load() -> Result<Config, String> {
    let _transaction = storage::transaction()?;
    load_at(&data_dir().join("config.json"))
}

/// 损坏时只回落内存中的默认值；不移动、删除或覆盖原文件。
pub fn load() -> Config {
    try_load().unwrap_or_else(|e| {
        eprintln!("vcc: 读取配置失败：{e}");
        Config::default()
    })
}

fn save_at(path: &Path, cfg: &Config) -> Result<(), String> {
    let _transaction = storage::transaction()?;
    let cfg = cfg.normalized()?;
    // 包括语义非法配置在内，必须先人工恢复，不能被默认值覆盖。
    load_at(path)?;
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    storage::atomic_write(path, &json)
}

pub fn save(cfg: &Config) -> Result<(), String> {
    save_at(&data_dir().join("config.json"), cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage::tests::TestDir;

    #[test]
    fn config_old_defaults_are_compatible() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(
            serde_json::to_value(&cfg).unwrap(),
            serde_json::to_value(Config::default()).unwrap()
        );
        assert!(cfg.review_transcript);
        assert!(!cfg.reduce_motion);
        assert!(cfg.validate().is_ok());
        let cfg: Config =
            serde_json::from_str(r#"{"review_transcript":false,"reduce_motion":true}"#).unwrap();
        assert!(!cfg.review_transcript);
        assert!(cfg.reduce_motion);
    }

    #[test]
    fn config_data_dir_override_without_environment_mutation() {
        assert_eq!(
            resolve_data_dir(Some("isolated".into()), Some("appdata".into())),
            PathBuf::from("isolated")
        );
        assert_eq!(
            resolve_data_dir(None, Some("appdata".into())),
            PathBuf::from("appdata").join("com.chidc.vcc")
        );
        assert_eq!(
            resolve_data_dir(Some("".into()), Some("appdata".into())),
            PathBuf::from("appdata").join("com.chidc.vcc")
        );
    }

    #[test]
    fn config_normalization_and_url_validation() {
        let cfg = Config {
            base_url: " https://EXAMPLE.com/v1/// ".into(),
            model: " model ".into(),
            hotkey: " Control + SHIFT + Space ".into(),
            voice_model: " QUALITY ".into(),
            voice_lang: " AUTO ".into(),
            theme: " LIGHT ".into(),
            ..Config::default()
        }
        .normalized()
        .unwrap();
        assert_eq!(cfg.base_url, "https://example.com/v1");
        assert_eq!(cfg.model, "model");
        assert_eq!(cfg.hotkey, "ctrl+shift+space");
        assert_eq!(cfg.voice_model, "quality");
        assert_eq!(cfg.voice_lang, "auto");
        assert_eq!(cfg.theme, "light");
        for url in [
            "http://localhost:8080/v1",
            "http://127.0.0.2",
            "http://[::1]:8000",
            "https://example.com/v1",
        ] {
            assert!(
                Config {
                    base_url: url.into(),
                    ..Config::default()
                }
                .validate()
                .is_ok(),
                "{url}"
            );
        }
        for url in [
            "",
            "example.com",
            "ftp://example.com",
            "http://example.com",
            "http://192.168.1.1",
            "http://localhost.example.com",
            "http://0.0.0.0",
            "https:///example.com",
            "https://example.com:bad",
            "https://user:pass@example.com",
            "https://example.com?q=1",
            "https://example.com#f",
            "https://exa mple.com",
            "https://example.com\r\n",
        ] {
            assert!(
                Config {
                    base_url: url.into(),
                    ..Config::default()
                }
                .validate()
                .is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn config_fields_and_hotkey_validation() {
        for field in [
            "model",
            "voice_model",
            "voice_lang",
            "theme",
            "hotkey",
            "api_key",
        ] {
            let mut value = serde_json::to_value(Config::default()).unwrap();
            value[field] = if field == "api_key" {
                "key\r\n".into()
            } else {
                " ".into()
            };
            let cfg: Config = serde_json::from_value(value).unwrap();
            assert!(cfg.validate().is_err(), "{field}");
        }
        for threads in [-1, 257, i32::MAX] {
            assert!(Config {
                voice_threads: threads,
                ..Config::default()
            }
            .validate()
            .is_err());
        }
        for threads in [0, 1, 256] {
            assert!(Config {
                voice_threads: threads,
                ..Config::default()
            }
            .validate()
            .is_ok());
        }
        for hotkey in [
            "ctrl",
            "ctrl++a",
            "ctrl+control+a",
            "a+b",
            "a+ctrl",
            "+a",
            "ctrl+a+",
            "f25",
            "ctrl+未知",
        ] {
            assert!(normalized_hotkey(hotkey).is_err(), "{hotkey}");
        }
        for hotkey in ["space", "f24", "ctrl+alt+k", "win+shift+9", "meta+escape"] {
            assert!(normalized_hotkey(hotkey).is_ok(), "{hotkey}");
        }
    }

    #[test]
    fn config_corrupt_and_invalid_files_are_read_only() {
        let dir = TestDir::new();
        let path = dir.0.join("config.json");
        for contents in ["{bad", r#"{"theme":"invalid","api_key":"keep"}"#] {
            fs::write(&path, contents).unwrap();
            assert!(load_at(&path).is_err());
            assert!(save_at(&path, &Config::default()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
            assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
        }
    }

    #[cfg(windows)]
    #[test]
    fn config_replace_failure_preserves_file_on_windows() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = TestDir::new();
        let path = dir.0.join("config.json");
        save_at(&path, &Config::default()).unwrap();
        let before = fs::read(&path).unwrap();
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        assert!(save_at(
            &path,
            &Config {
                reduce_motion: true,
                ..Config::default()
            }
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
        drop(held);
        save_at(
            &path,
            &Config {
                reduce_motion: true,
                ..Config::default()
            },
        )
        .unwrap();
        assert!(load_at(&path).unwrap().reduce_motion);
    }

    #[test]
    fn config_save_round_trip_and_reject_invalid_update() {
        let dir = TestDir::new();
        let path = dir.0.join("config.json");
        let cfg = Config {
            api_key: "keep".into(),
            ..Config::default()
        };
        save_at(&path, &cfg).unwrap();
        let before = fs::read(&path).unwrap();
        assert_eq!(load_at(&path).unwrap().api_key, "keep");
        assert!(save_at(
            &path,
            &Config {
                model: "".into(),
                ..cfg
            }
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        save_at(
            &path,
            &Config {
                reduce_motion: true,
                ..Config::default()
            },
        )
        .unwrap();
        assert!(load_at(&path).unwrap().reduce_motion);
    }
}
