# 开发指南

面向想自己编译、二次开发或提交 PR 的同学。产品视角请看 [README](../README.md)。

## 架构总览

```
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
`hide_main` `hide_floating` `set_autostart` `set_always_on_top` `probe_env`。

## 构建

依赖：Rust stable-msvc、Node ≥ 20、Python 3.12+（仅测试脚本）、NSIS（tauri 自动带）。

```bash
npm install            # 若存在 package.json；纯 tauri cli 可 npx tauri 直接跑
npx tauri build        # 产出 src-tauri/target/release/bundle/nsis/*-setup.exe
npx tauri dev          # 开发模式热重载
```

whisper 模型：`tools/models/ggml-small.bin`（本地放置，不入 git）。
首次运行请点主窗 ⚙ 填入 DeepSeek API Key（存于 `%APPDATA%\com.chidc.vcc\config.json`）。

## 测试

```bash
cd src-tauri && cargo test --release
# tools::safety::*   危险命令拦截回归（新增拦截模式请同步补测试）
# tools::tests::*    音量/文件读写真机往返
```

前端键盘逻辑（输入历史等）可用 Playwright 驱动 http.server 页面验证：
`scripts/test-history.js`（先 `python -m http.server 8137` 于仓库根）。

## E2E / 截图管线（scripts/）

| 脚本 | 用途 |
|---|---|
| `e2e-final.py` | `VCC_DEMO=1` 真机全链路三连拍：listening → done 峰值 → 淡出闭环 |
| `snap-ui.js`   | Playwright 批量截 UI 各状态（浏览器渲染，迭代视觉用） |
| `perf-vcc.py`  | 进程树 CPU/内存采样（psutil），验证光环渲染成本 |
| `make-icon.py` | PIL 程序化生成全套应用图标 |
| `test-history.js` | 输入历史键盘级验证 |

**VCC_DEMO=1** 是应用内置的测试钩子（`lib.rs` setup）：启动 1.2s 后自动执行一次
完整呼出链路（show 主窗 → `vcc://invoked` → 悬浮窗步骤卡 → done），不设置则完全无感。
选它而不是模拟热键的原因：SendInput→RegisterHotKey 合成输入分发在某些安全软件下不可靠。

踩坑备忘：
- Tauri 2 release 前端资源**编译期嵌入**——改 `ui/` 后必须重跑 `npx tauri build`
- Chromium 对 http.server 页面有 304 缓存——截图脚本统一 `cb=Date.now()` 破缓存
- 光环 done 阶段仅 900ms，E2E 抓峰值要在窗口内截图

## 项目约定

- 主窗**黑/白极简**：只有全屏光环与输入条扫光允许彩色，其余动效一律白色系
- Apple 风：SF 字体栈、毛玻璃、克制动效；所有动效尊重 `prefers-reduced-motion`
- 危险命令（删除/格式化/关机/提权/远程执行）在 `is_blocked` 硬拦截，课堂场景零容忍
- 本地资源不入 git：`tools/whisper`、`tools/models`、`generated-images`
- 提交信息：`feat|fix|docs|chore|overlay: 摘要`，一个逻辑一个提交
