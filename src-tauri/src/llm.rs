use crate::{config::Config, tools};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

/* ---------- DeepSeek / OpenAI 兼容消息结构 ---------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuncCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FuncCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn text(role: &str, content: &str) -> Self {
        Self { role: role.into(), content: Some(content.into()), tool_calls: None, tool_call_id: None }
    }
}

/* ---------- 工具 schema ---------- */

fn tool(name: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": desc,
            "parameters": {
                "type": "object",
                "properties": props,
                "required": required
            }
        }
    })
}

fn tool_defs() -> Vec<Value> {
    // 精简 schema：每次请求都要携带，token 越少首 token 延迟越低（课堂单轮指令场景够用）
    vec![
        tool("set_volume", "设置音量0-100", json!({"level": {"type": "number"}}), &["level"]),
        tool("adjust_volume", "相对调音量，如大点声+15", json!({"delta": {"type": "number"}}), &["delta"]),
        tool("get_volume", "查询音量", json!({}), &[]),
        tool("toggle_mute", "切换静音", json!({}), &[]),
        tool("set_brightness", "设置亮度0-100（仅内置屏）", json!({"level": {"type": "number"}}), &["level"]),
        tool("mouse_move", "移动鼠标到绝对像素", json!({"x": {"type": "integer"}, "y": {"type": "integer"}}), &["x", "y"]),
        tool("mouse_click", "点击（可选先移动/右键/双击）", json!({
            "x": {"type": "integer"},
            "y": {"type": "integer"},
            "button": {"type": "string", "enum": ["left", "right"]},
            "double": {"type": "boolean"}
        }), &[]),
        tool("mouse_drag", "按住左键拖动", json!({
            "from_x": {"type": "integer"}, "from_y": {"type": "integer"},
            "to_x": {"type": "integer"}, "to_y": {"type": "integer"}
        }), &["from_x", "from_y", "to_x", "to_y"]),
        tool("scroll_wheel", "滚轮滚动", json!({
            "direction": {"type": "string", "enum": ["up", "down"]},
            "clicks": {"type": "integer"}
        }), &["direction"]),
        tool("type_text", "在焦点窗口输入文字", json!({"text": {"type": "string"}}), &["text"]),
        tool("press_hotkey", "按组合键如 ctrl+s、alt+tab、win+d", json!({"keys": {"type": "string"}}), &["keys"]),
        tool("run_command", "运行 PowerShell 命令（禁删除/格式化/关机）", json!({"command": {"type": "string"}}), &["command"]),
        tool("open_path", "打开文件/文件夹/网址", json!({"path": {"type": "string"}}), &["path"]),
        tool("open_app", "按名称启动应用如 notepad、calc", json!({"name": {"type": "string"}}), &["name"]),
        tool("list_dir", "列出目录", json!({"path": {"type": "string"}}), &["path"]),
        tool("read_file", "读文本文件", json!({"path": {"type": "string"}}), &["path"]),
        tool("write_file", "写文本文件（append=true 追加）", json!({
            "path": {"type": "string"},
            "content": {"type": "string"},
            "append": {"type": "boolean"}
        }), &["path", "content"]),
        tool("search_files", "按名递归搜文件", json!({"dir": {"type": "string"}, "pattern": {"type": "string"}}), &["dir", "pattern"]),
        tool("screenshot", "截取全部屏幕并打开截图", json!({}), &[]),
        tool("read_screen", "读取屏幕：列出前台/指定窗口的可点击元素与中心坐标（模型看不了截图，操作界面必须先看这个）", json!({
            "window": {"type": "string", "description": "窗口标题关键字；缺省=当前前台；all=列出全部可见窗口"}
        }), &[]),
        tool("ocr_screen", "OCR 识别屏幕或窗口图片里的文字（read_screen 读不到的图片/自绘界面用这个兜底）", json!({
            "window": {"type": "string", "description": "窗口标题关键字；缺省=整个屏幕"}
        }), &[]),
        tool("show_dialog", "弹出确认对话框并等待用户选择，返回用户点了哪个按钮（用于需要用户确认/二选一/授权时）", json!({
            "title": {"type": "string"},
            "body": {"type": "string"},
            "buttons": {"type": "array", "description": "1-4 个按钮", "items": {"type": "object", "properties": {
                "label": {"type": "string"},
                "style": {"type": "string", "enum": ["normal", "primary", "danger"]}
            }}},
            "timeout_secs": {"type": "integer", "description": "超时自动关闭，缺省 120"}
        }), &["body"]),
        tool("clipboard", "读取或写入剪贴板文本", json!({
            "action": {"type": "string", "enum": ["get", "set"]},
            "text": {"type": "string", "description": "action=set 时必填"}
        }), &["action"]),
    ]
}

fn tool_label(name: &str, args: &Value) -> String {
    let g = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let gn = |k: &str| args.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
    match name {
        "set_volume" => format!("调节音量至 {}%", gn("level")),
        "adjust_volume" => format!("音量{}{}", if gn("delta") >= 0.0 { "+" } else { "" }, gn("delta")),
        "get_volume" => "查询音量".into(),
        "toggle_mute" => "切换静音".into(),
        "set_brightness" => format!("调节亮度至 {}%", gn("level")),
        "mouse_move" => format!("移动鼠标 ({}, {})", gn("x"), gn("y")),
        "mouse_click" => {
            let t = if args.get("double").and_then(|d| d.as_bool()).unwrap_or(false) { "双击" } else { "点击" };
            format!("鼠标{t}")
        }
        "mouse_drag" => "拖动鼠标".into(),
        "scroll_wheel" => format!("{}滚动", if g("direction") == "up" { "向上" } else { "向下" }),
        "type_text" => "输入文字".into(),
        "press_hotkey" => format!("按键 {}", g("keys")),
        "run_command" => format!("运行命令 · {}", g("command")),
        "open_path" => format!("打开 {}", g("path")),
        "open_app" => format!("启动 {}", g("name")),
        "list_dir" => format!("浏览 {}", g("path")),
        "read_file" => format!("读取 {}", g("path")),
        "write_file" => format!("写入 {}", g("path")),
        "search_files" => format!("搜索「{}」", g("pattern")),
        "screenshot" => "截取屏幕".into(),
        "read_screen" => "读取屏幕元素".into(),
        "ocr_screen" => {
            let w = g("window");
            if w.is_empty() { "OCR 识别屏幕文字".into() } else { format!("OCR 识别「{w}」") }
        }
        "show_dialog" => format!("弹窗提问 · {}", g("title")),
        "clipboard" => {
            if g("action") == "set" { "复制到剪贴板".into() } else { "读取剪贴板".into() }
        }
        _ => format!("执行 {name}"),
    }
}

/* ---------- System Prompt ---------- */

fn build_system_prompt(memory: &str) -> String {
    let (w, h) = tools::screen_size();
    let now = chrono_now_cn();
    // 长期记忆注入：截断 800 字防止 system prompt 膨胀
    let mem_block = if memory.trim().is_empty() {
        String::new()
    } else {
        let mem: String = memory.chars().take(800).collect();
        format!("\n9. 用户长期记忆（跨会话积累，供个性化参考）：\n{}", mem)
    };
    format!(
        "你是 VCC（Voice Control for Class），运行在 Windows 电脑上的课堂助手 Agent，用户通过语音或文字让你操作这台电脑。\n\
规则：\n\
1. 一律用简体中文回答，简短直接（一两句话），不要客套。\n\
2. 凡是能通过工具完成的操作，必须调用工具实际执行，绝不口头假装。\n\
3. 多个操作按顺序逐个调用工具。\n\
4. 鼠标坐标是屏幕绝对像素。屏幕分辨率：{w}x{h}。你看不到截图：操作任何应用界面时，必须先用 read_screen 拿到按钮/菜单/输入框的中心坐标，再用 mouse_click 点击、type_text 输入、press_hotkey 按键；不知道元素位置时禁止盲点。read_screen 读不到的图片/自绘界面用 ocr_screen 识别文字。\n\
5. 需要用户确认、授权或二选一时，用 show_dialog 弹窗（自定义标题/正文/按钮，危险操作用 danger 按钮），等它返回用户的选择再继续；不要自问自答。\n\
6. 当前时间：{now}。\n\
7. 禁止执行删除文件、格式化、关机类命令；用户要求时简短说明这是课堂安全限制。\n\
8. 执行完操作后简单报告结果即可，不要复述细节。{mem_block}"
    )
}

pub fn chrono_now_cn() -> String {
    // 无 chrono 依赖，用标准库取本机时间
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    // 东八区 + 8h，换算日期
    let secs = now + 8 * 3600;
    let days = secs / 86400;
    let tod = secs % 86400;
    let (h, m) = (tod / 3600, (tod % 3600) / 60);
    // civil_from_days 算法
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y}年{mo}月{d}日 {h:02}:{m:02}")
}

/* ---------- API 调用（SSE 流式） ---------- */

async fn chat_completion_stream(
    cfg: &Config,
    messages: &[ChatMessage],
    app: &AppHandle,
) -> Result<ChatMessage, String> {
    use futures_util::StreamExt;

    if cfg.api_key.is_empty() {
        return Err("未配置 API Key，请点主窗口右上角 ⚙ 填入 DeepSeek API Key".into());
    }
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    let body = json!({
        "model": cfg.model,
        "messages": messages,
        "tools": tool_defs(),
        "tool_choice": "auto",
        "temperature": 0.3,
        "stream": true
    });

    let resp = client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "API 错误 {status}: {}",
            text.chars().take(300).collect::<String>()
        ));
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut content_acc = String::new();
    let mut streaming = false;
    // delta 合并：33ms 批量 emit（DeepSeek 每 chunk 1-3 字太碎，直发会打爆 IPC）
    let mut delta_buf = String::new();
    let mut last_flush = std::time::Instant::now();
    // tool_call index -> (id, name, arguments)
    let mut tools_acc: std::collections::BTreeMap<usize, (String, String, String)> =
        std::collections::BTreeMap::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| format!("流中断: {e}"))?;
        buf.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(pos) = buf.find("\n\n") {
            let event: String = buf.drain(..pos + 2).collect();
            for line in event.lines() {
                let data = match line.strip_prefix("data:") {
                    Some(d) => d.trim(),
                    None => continue,
                };
                if data == "[DONE]" {
                    continue;
                }
                let v: Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let delta = &v["choices"][0]["delta"];

                if let Some(c) = delta["content"].as_str() {
                    if !c.is_empty() {
                        if !streaming {
                            streaming = true;
                            let _ = app.emit("vcc://chat-start", json!({}));
                        }
                        content_acc.push_str(c);
                        delta_buf.push_str(c);
                        if last_flush.elapsed() >= std::time::Duration::from_millis(33) {
                            let _ = app.emit("vcc://chat-delta", json!({"text": delta_buf}));
                            delta_buf = String::new();
                            last_flush = std::time::Instant::now();
                        }
                    }
                }

                if let Some(tcs) = delta["tool_calls"].as_array() {
                    for tc in tcs {
                        let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                        let entry = tools_acc.entry(idx).or_default();
                        if let Some(id) = tc["id"].as_str() {
                            entry.0 = id.to_string();
                        }
                        if let Some(f) = tc["function"].as_object() {
                            if let Some(n) = f.get("name").and_then(|x| x.as_str()) {
                                if !n.is_empty() {
                                    entry.1.push_str(n);
                                }
                            }
                            if let Some(a) = f.get("arguments").and_then(|x| x.as_str()) {
                                entry.2.push_str(a);
                            }
                        }
                    }
                }
            }
        }
    }

    // 冲刷剩余 delta（流尾不足 33ms 的尾巴）
    if !delta_buf.is_empty() {
        let _ = app.emit("vcc://chat-delta", json!({"text": delta_buf}));
    }
    let _ = app.emit("vcc://chat-end", json!({}));

    let tool_calls = if tools_acc.is_empty() {
        None
    } else {
        let mut v = Vec::new();
        for (i, (id, name, arguments)) in tools_acc {
            v.push(ToolCall {
                id: if id.is_empty() { format!("call_{i}") } else { id },
                kind: "function".into(),
                function: FuncCall {
                    name,
                    arguments: if arguments.is_empty() { "{}".into() } else { arguments },
                },
            });
        }
        Some(v)
    };

    Ok(ChatMessage {
        role: "assistant".into(),
        content: if content_acc.is_empty() { None } else { Some(content_acc) },
        tool_calls,
        tool_call_id: None,
    })
}

/* ---------- 非流式补全（记忆总结等后台任务用） ---------- */

pub async fn chat_completion_simple(cfg: &Config, messages: &[ChatMessage]) -> Result<String, String> {
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let body = json!({
        "model": cfg.model,
        "messages": messages,
        "temperature": 0.3,
        "stream": false
    });
    let resp = client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "API 错误 {status}: {}",
            text.chars().take(200).collect::<String>()
        ));
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

/* ---------- Agent 主循环 ---------- */

pub async fn run_agent(app: &AppHandle, text: String) -> Result<(), String> {
    let cfg = crate::config::load(app);
    let memory = crate::memory::load_memory(app);

    let history = {
        let state = app.state::<crate::AppState>();
        let mut hist = state.history.lock().map_err(|_| "历史锁错误")?;
        hist.push(ChatMessage::text("user", &text));
        if hist.len() > 60 {
            // 上下文压缩：只保留最近 60 条，被裁掉的旧消息后台并入长期记忆
            let l = hist.len();
            let dropped: Vec<ChatMessage> = hist.drain(..l - 60).collect();
            if !dropped.is_empty() {
                let app2 = app.clone();
                tauri::async_runtime::spawn(async move {
                    crate::memory::summarize_into_memory(app2, dropped).await;
                });
            }
        }
        hist.clone()
    };

    let _ = app.emit("vcc://phase", json!({"phase": "thinking"}));

    let mut messages: Vec<ChatMessage> =
        vec![ChatMessage::text("system", &build_system_prompt(&memory))];
    messages.extend(history);

    let mut steps: Vec<Value> = Vec::new();
    let mut rounds = 0;

    loop {
        rounds += 1;
        if rounds > 10 {
            return Err("这个任务步骤太多了，试试拆成两步告诉我".into());
        }

        let msg = chat_completion_stream(&cfg, &messages, app).await?;

        if let Some(tcs) = &msg.tool_calls {
            if !tcs.is_empty() {
                messages.push(msg.clone());
                for tc in tcs {
                    let label = tool_label(&tc.function.name, &serde_json::from_str::<Value>(&tc.function.arguments).unwrap_or_else(|_| json!({})));
                    steps.push(json!({"label": label, "status": "running"}));

                    let _ = app.emit("vcc://phase", json!({"phase": "executing"}));
                    let _ = app.emit("vcc://tool", json!({"id": tc.id, "label": label}));
                    let _ = app.emit("vcc://float", json!({"mode": "show", "steps": steps, "state": "执行中"}));

                    let result = tools::execute(&tc.function.name, &tc.function.arguments).await;
                    let ok = result.is_ok();
                    let content = result.unwrap_or_else(|e| format!("错误: {e}"));
                    // 工具结果截断：长输出（命令/文件内容）只留前 1500 字，防上下文膨胀拖慢请求
                    let content = if content.chars().count() > 1500 {
                        let head: String = content.chars().take(1500).collect();
                        format!("{head}\n…（结果过长已截断）")
                    } else {
                        content
                    };

                    steps.last_mut().unwrap()["status"] = json!(if ok { "done" } else { "fail" });
                    let _ = app.emit("vcc://tool-done", json!({"id": tc.id, "ok": ok}));
                    let _ = app.emit("vcc://float", json!({"mode": "show", "steps": steps, "state": "执行中"}));

                    messages.push(ChatMessage {
                        role: "tool".into(),
                        content: Some(content),
                        tool_calls: None,
                        tool_call_id: Some(tc.id.clone()),
                    });
                }
                continue;
            }
        }

        // 最终回答（内容已通过流式事件渲染到前端）
        let answer = msg.content.unwrap_or_else(|| "（已完成）".into());
        {
            let state = app.state::<crate::AppState>();
            let mut hist = state.history.lock().map_err(|_| "历史锁错误")?;
            hist.push(ChatMessage::text("assistant", &answer));
            // 落盘：重启后恢复对话流
            crate::memory::save_history(app, &hist);
        }
        // 纯工具调用无文本输出时，主窗口补一个完成气泡
        if answer.trim().is_empty() || answer == "（已完成）" {
            let _ = app.emit("vcc://chat", json!({"role": "ai", "text": "✓ 已执行完成"}));
        }
        let _ = app.emit("vcc://float", json!({"mode": "done", "steps": steps, "text": answer}));
        let _ = app.emit("vcc://phase", json!({"phase": "idle"}));
        return Ok(());
    }
}
