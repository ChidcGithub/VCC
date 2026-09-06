# -*- coding: utf-8 -*-
"""CHANGELOG 补充 2026-09-06 全量审查条目"""
import io

p = 'CHANGELOG.md'
s = io.open(p, encoding='utf-8').read()
marker = '## [Unreleased]'
assert marker in s
block = '''## [Unreleased]

### 全量 Bug 审查与修复（子代理三路并行审查：前端 JS / Rust 后端 / 资源一致性）

**核心时序与并发**
- agent_run 改为直接 await（此前 spawn 后立即返回，invoke 几毫秒就 resolve，前端 agentBusy 门禁/完成绽放/会话刷新全部拿到错误时序）
- 会话命令（reset/switch/delete/clear）busy 检查改 CAS 占位 + BusyGuard 自动释放（check-then-act 竞态窗口）；rename_session 补上遗漏的 busy 检查
- chat-end 仅在最终回答轮发一次（此前每轮都发，多步任务期间 phase 在 done/idle/executing 间抖动）；错误路径补发
- run_agent 失败回滚本次 user 消息（重试不再产生重复轮次，内存与磁盘一致）
- AI 记忆总结单飞（并发触发时后写者会抹掉先写者的成果）

**数据安全**
- sessions.json / memory.json / config.json 全部改原子写（临时文件 + rename），进程中途被杀不再留半截 JSON
- 三个 JSON 损坏时保留 .bad 现场再回落默认（此前 sessions 损坏被空文件覆盖=全部会话不可逆丢失；config 损坏则 API Key 被静默抹掉）
- 上下文裁剪不再切断 assistant(tool_calls) 与 tool 的配对（切断会让 API 持续 400，会话报废）
- SSE 流改字节缓冲、事件边界整段解码（中文被 TCP 分包切开时不再产生 U+FFFD 乱码）

**安全拦截**
- run_command 黑名单补编码执行通道（-EncodedCommand/-ec/FromBase64String 此前可整体绕过）+ .exe 后缀归一化（reg.exe add 此前绕过）

**资源泄漏**
- 命令执行/OCR 超时改 kill_on_drop 杀子进程（此前超时只弃 future，孤儿进程继续执行）
- show_dialog 兜底超时文案与事实对齐（弹窗实际未关闭时不再谎报「已自动关闭」）

**前端**
- 录音竞态修复：双击不再泄漏麦克风流；授权等待期松手即撤销意图
- agent 忙时语音识别文本放回输入条（此前静默吞掉）
- 新对话被后端拒绝时保留当前对话（此前清掉正在流式输出的气泡）
- 重试按钮跨会话不再重发旧指令；设置面板 probe_env 加代际保护
- 浅色主题用户首帧预读（localStorage，消除启动闪深色）
- 删除 vcc://transcript 死监听与 preview 死类；悬浮窗 hide 计时器纳入跟踪（淡出窗口期新任务不被误杀）；icons.css 重复引入去重

### 修复：保存设置报「保存失败：注册表乱码错误」'''
if '全量 Bug 审查与修复' not in s:
    s = s.replace(marker, block, 1)
    io.open(p, 'w', encoding='utf-8', newline='').write(s)
    print('CHANGELOG written')
else:
    print('already present')
