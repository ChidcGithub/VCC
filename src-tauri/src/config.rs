use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

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
    /// 朗读 AI 回答（投影课堂：学生听得到回答）
    #[serde(default)]
    pub tts_enabled: bool,
    /// 界面主题：dark / light（body class 切换，对齐 chat.deepseek.com 机制）
    #[serde(default = "default_theme")]
    pub theme: String,
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
        }
    }
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|_| "无法定位配置目录".to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("config.json"))
}

/// 读取配置；解析失败时把坏文件改名 .bad 保留现场再回落默认
/// （否则下次 save 用默认配置覆盖，API Key 被静默抹掉）
pub fn load(app: &AppHandle) -> Config {
    let parsed = config_path(app).ok().and_then(|p| {
        fs::read_to_string(&p)
            .ok()
            .map(|s| (p, serde_json::from_str::<Config>(&s).ok()))
    });
    match parsed {
        Some((_, Some(cfg))) => cfg,
        Some((p, None)) => {
            let _ = fs::rename(&p, p.with_extension("json.bad"));
            Config::default()
        }
        None => Config::default(),
    }
}

pub fn save(app: &AppHandle, cfg: &Config) -> Result<(), String> {
    let path = config_path(app)?;
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    // 原子写：临时文件 + rename 替换（进程中途被杀不留半截 JSON，
    // 半截 JSON 会让下次 load 回落默认配置、API Key 被静默抹掉）
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}
