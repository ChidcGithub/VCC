# SignPath Foundation 申请材料

> 用途：在 https://signpath.org/ 点击 "Apply for Free Code Signing" 后，把下面各栏粘贴进去。
> 前置条件自查：☑ 公开仓库 ☑ OSI 许可证（MIT）☑ 已发布 Release（含安装包）☑ README 含功能描述 / 隐私政策 / 卸载说明

---

## Project name

Voice Control for Class (VCC)

## Project URL (repository)

https://github.com/chidc/voice-control-for-class
<!-- 创建仓库后按实际 URL 修改 -->

## Download page URL

https://github.com/chidc/voice-control-for-class/releases
<!-- v0.1.0 Release 附 NSIS 安装包后即为正式下载页 -->

## Project description (英文，可直接粘贴)

Voice Control for Class (VCC) is an open-source, LLM-powered desktop assistant for classroom
computers, built for teachers and students. Press a global hotkey (Ctrl+Shift+Space), then speak
or type a natural-language command, and an AI agent directly controls the PC: adjusting volume and
brightness, moving / clicking / dragging the mouse, typing and sending key combinations, browsing
and editing files, opening apps and URLs, and running whitelisted PowerShell commands (with
hard-blocked destructive patterns such as delete / format / shutdown).

The application is written in Rust with Tauri 2 and a dependency-free HTML/CSS/JS frontend.
It uses DeepSeek function calling (OpenAI-compatible API, configurable) over an SSE streaming
agent loop, and performs speech-to-text entirely offline and locally via whisper.cpp — audio is
never uploaded. A system-tray mode keeps the app in the background; results are shown in a
translucent overlay window that fades out after five seconds.

The Windows installer (NSIS) and portable binary are published on the Releases page. The project
is licensed under the MIT license and maintained by a single student developer; the repository
contains full build instructions (Rust 1.77+ / Node 18+ required).

## What license does the project use?

MIT License (see LICENSE in the repository root)

## Who will sign binaries?

The project maintainer (Chidc) — single-maintainer team. Signing requests are approved by the
same maintainer via the SignPath CI pipeline (GitHub Actions) as part of the release workflow.

## 功能描述（中文备份，审核界面可能用到）

大模型驱动的课堂电脑助手：全局热键呼出，语音/文字指令驱动 AI 直接控制电脑（音量、亮度、鼠标、
键盘、文件、PowerShell），本地 whisper.cpp 离线转写，SSE 流式回答，托盘常驻 + 悬浮结果窗。

---

## 申请后要做的事（SignPath 通过审核之后）

1. 注册 signpath.io 账号（用 GitHub 登录），Foundation 会开通免费 OSS 订阅
2. 在 SignPath 后台创建 Signing Policy + 连接 GitHub 仓库（GitHub App 授权）
3. 在仓库加 GitHub Actions workflow：build → 产物提交 SignPath 签名 → 签名后传 Release
4. 团队角色：自己同时是 Author / Approver（条款要求 MFA，GitHub 开 2FA）
5. tauri build 保持产物未签名 → 由 pipeline 在 release 阶段签名（不要在本地签）

## 注意事项（对照 Foundation 条款）

- 条款禁止 "hacking tools" —— VCC 的输入模拟能力是正常辅助功能，描述里强调 classroom assistant 定位即可
- 条款要求收集数据必须有隐私政策 —— README「隐私说明」一节已覆盖（麦克风本地处理、无遥测）
- 条款要求提供卸载方式 —— README「卸载」一节已覆盖
- 条款要求代码签名政策公开 —— 申请通过后在 README 加一节 "Code signing policy"
