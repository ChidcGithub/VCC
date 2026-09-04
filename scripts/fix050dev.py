# -*- coding: utf-8 -*-
"""DEVELOPMENT.md 架构区对齐 v0.5.0"""
import io

P = r"D:\My things\Learn\高二\VCC\docs\DEVELOPMENT.md"
src = io.open(P, encoding="utf-8").read()

old = """```
┌──────────────────────── Rust（src-tauri/src/）────────────────────────┐
│  lib.rs        入口：三窗口创建、托盘、全局热键、VCC_DEMO 钩子、      │
│                自启/置顶/收起命令、vcc://float → 悬浮窗显示监听         │
│  llm.rs        Agent 循环：DeepSeek function calling（SSE 流式），     │
│                工具轮次（最多 N 轮），vcc://chat-start/delta/end 事件   │
│  tools.rs      17 个系统控制工具 + is_blocked() 危险命令拦截（有单测） │
│  voice.rs      whisper.cpp 转写封装 + 环境探测（probe_env）            │
│  config.rs     Config（serde）：Key/URL/模型/热键/自启/置顶            │
└───────────────────────────────────────────────────────────────────┘
            │ vcc://* 事件（emit）            ▲ invoke（前端调用）
┌───────────▼─────────────────── 前端（ui/，原生 HTML/CSS/JS）─────────┐
│  index.html + main.js + style.css     主窗：对话、录音 WAV 编码、      │
│                                       AnalyserNode 语音电平泵          │
│  overlay.html + overlay.js            全屏光环：WebGL 七色双层光带、   │
│                                       uSpin 环流、语音响应、done 绽放  │
│  floating.html/js/css                 右上角悬浮步骤卡，done 后 5s 淡出 │
└───────────────────────────────────────────────────────────────────┘
```

事件契约：`vcc://invoked`（呼出）`vcc://level`（语音电平 0-1）`vcc://phase`（阶段）
`vcc://chat-start/delta/end`（流式）`vcc://chat`（整段消息）`vcc://float`（悬浮窗渲染）。"""

new = """```
┌──────────────────────── Rust（src-tauri/src/）────────────────────────┐
│  lib.rs        入口：三窗口创建、托盘、热键动态注册（配置生效/回退）、  │
│                server 预热线程（启动 20s）、空闲回收线程（30min）、      │
│                单实例保护、agent busy 并发锁、VCC_DEMO/VCC_BENCH 钩子   │
│  llm.rs        Agent 循环：DeepSeek function calling（SSE 流式，        │
│                delta 33ms 批量 emit），工具轮次（≤10 轮），              │
│                记忆注入 system prompt、结果截断 1500 字                  │
│  tools.rs      19 个系统控制工具 + is_blocked() 危险命令拦截（有单测） │
│  voice.rs      whisper-server 常驻（跨实例复用/两连失败降级 CLI）、     │
│                预热 warmup()、空闲回收 start_idle_reaper()、线程自适应  │
│  memory.rs     history.json 持久化 + memory.json 长期记忆 +             │
│                summarize_into_memory 后台总结（失败静默）               │
│  config.rs     Config（serde，全部字段带默认值）：Key/URL/模型/热键/    │
│                自启/置顶/语音模型/线程/语言/快捷指令/TTS                │
└───────────────────────────────────────────────────────────────────┘
            │ vcc://* 事件（emit）            ▲ invoke（前端调用）
┌───────────▼─────────────────── 前端（ui/，原生 HTML/CSS/JS）─────────┐
│  index.html + main.js + style.css     主窗：对话（轻量 markdown）、    │
│                                       录音 WAV 编码 + 静音裁剪 + 60s   │
│                                       上限、TTS 朗读、空态快捷指令、   │
│                                       智能滚动、文件拖拽               │
│  overlay.html + overlay.js            全屏光环：WebGL 七色双层光带、   │
│                                       uSpin 环流、语音响应、done 绽放  │
│  floating.html/js/css                 右上角悬浮步骤卡，done 后 5s 淡出 │
└───────────────────────────────────────────────────────────────────┘
```

事件契约：`vcc://invoked`（呼出）`vcc://level`（语音电平 0-1）`vcc://phase`（阶段）
`vcc://chat-start/delta/end`（流式）`vcc://chat`（整段消息）`vcc://float`（悬浮窗渲染）
`vcc://tool` / `vcc://tool-done`（工具行计时）`vcc://memory-updated`（记忆归档提示）
`vcc://transcript`（悬浮识别文字）。

命令契约：`agent_run` `transcribe` `get/save_config` `load_chat_history`
`get_memory` `save_memory_cmd` `reset_history`（先归档精华再清空）
`hide_main` `hide_floating` `set_autostart` `set_always_on_top` `probe_env`。"""

assert old in src, "architecture block missing"
src = src.replace(old, new, 1)
io.open(P, "w", encoding="utf-8", newline="\n").write(src)
print("DEVELOPMENT.md updated")
