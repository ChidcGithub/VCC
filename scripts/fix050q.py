# -*- coding: utf-8 -*-
"""批次F：消息时间戳（hover 可见）+ 聊天智能滚动（回看历史不被强拉到底）"""
import io, sys
fails = []

def patch(path, pairs):
    src = io.open(path, encoding="utf-8").read()
    for i, (old, new) in enumerate(pairs):
        if old in src:
            src = src.replace(old, new, 1)
            print(f"  {path.split(chr(92))[-1]} #{i+1}: OK")
        else:
            fails.append(f"{path} #{i+1}")
            print(f"  {path.split(chr(92))[-1]} #{i+1}: NOT FOUND")
    io.open(path, "w", encoding="utf-8", newline="\n").write(src)

B = r"D:\My things\Learn\高二\VCC"

# ============ main.js ============
patch(B + r"\ui\main.js", [
    # 1. 智能滚动 + 时间戳 helper（挂在 addBubble 前）
    ("""/* AI 回答轻量 markdown：**bold** / `code`。先整体 HTML 转义再替换，杜绝注入 */""",
     """/* 智能滚动：仅当本来就贴近底部时跟随新消息（回看历史不被强拉走） */
function scrollChat(force) {
  const nearBottom = chatEl.scrollHeight - chatEl.scrollTop - chatEl.clientHeight < 90;
  if (force || nearBottom) chatEl.scrollTop = chatEl.scrollHeight;
}

function nowHM() {
  const d = new Date();
  return String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
}

/* AI 回答轻量 markdown：**bold** / `code`。先整体 HTML 转义再替换，杜绝注入 */"""),
    # 2. addBubble 加 title 时间戳 + 用 scrollChat
    ("""function addBubble(role, text) {
  const div = document.createElement('div');
  div.className = 'bubble ' + role;
  if (role === 'ai') div.innerHTML = renderInlineMd(text);
  else div.textContent = text;
  chatEl.appendChild(div);
  chatEl.scrollTop = chatEl.scrollHeight;
  return div;
}""",
     """function addBubble(role, text) {
  const div = document.createElement('div');
  div.className = 'bubble ' + role;
  if (role === 'ai') div.innerHTML = renderInlineMd(text);
  else div.textContent = text;
  div.title = nowHM();
  chatEl.appendChild(div);
  scrollChat(role === 'user');
  return div;
}"""),
    # 3. 其余强滚点保持 force（历史恢复、分隔线、工具行）
    ("""  div.className = 'chat-divider';
  div.textContent = text;
  chatEl.appendChild(div);
  chatEl.scrollTop = chatEl.scrollHeight;
}""",
     """  div.className = 'chat-divider';
  div.textContent = text;
  chatEl.appendChild(div);
  scrollChat(true);
}"""),
    ("""  div.querySelector('.t-label').textContent = label;
  chatEl.appendChild(div);
  chatEl.scrollTop = chatEl.scrollHeight;
  return div;
}""",
     """  div.querySelector('.t-label').textContent = label;
  div.title = nowHM();
  chatEl.appendChild(div);
  scrollChat(true);
  return div;
}"""),
    # 4. 流式 delta 跟随（贴底才滚）
    ("""listen('vcc://chat-delta', (e) => {
  if (!streamBubble) streamBubble = addBubble('ai', '');
  streamBubble.textContent += e.payload.text;
  chatEl.scrollTop = chatEl.scrollHeight;
});""",
     """listen('vcc://chat-delta', (e) => {
  if (!streamBubble) streamBubble = addBubble('ai', '');
  streamBubble.textContent += e.payload.text;
  scrollChat(false);
});"""),
    # 5. err 气泡滚到底（错误必须被看见）
    ("""  div.appendChild(btn);
  chatEl.appendChild(div);
  chatEl.scrollTop = chatEl.scrollHeight;
}

function hideEmptyState() {""",
     """  if (btn) div.appendChild(btn);
  chatEl.appendChild(div);
  scrollChat(true);
}

function hideEmptyState() {"""),
])

if fails:
    print("FAILED:", fails); sys.exit(1)
print("ALL OK")
