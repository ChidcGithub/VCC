# -*- coding: utf-8 -*-
"""一次性补丁：M3 图标替换（Edit 工具假成功，改用确定性脚本）。
每个替换先断言 old 出现且唯一，改完打印验证计数。幂等：已替换过则跳过。"""
import io, sys

ROOT = r"D:\My things\Learn\高二\VCC\ui"

def patch(path, pairs):
    p = ROOT + "\\" + path
    s = io.open(p, encoding="utf-8").read()
    changed = 0
    for old, new in pairs:
        if new in s and old not in s:
            continue  # 已是目标状态（幂等）
        n = s.count(old)
        assert n == 1, "%s: old 出现 %d 次（应为 1）: %r" % (path, n, old[:60])
        s = s.replace(old, new)
        changed += 1
    io.open(p, "w", encoding="utf-8", newline="").write(s)
    print("%s: %d 处替换" % (path, changed))

# ---------------- index.html ----------------
patch("index.html", [
    # 0. icons.css 引入
    ('<link rel="stylesheet" href="style.css" />',
     '<link rel="stylesheet" href="icons.css" />\n  <link rel="stylesheet" href="style.css" />'),
    # 1. 收起侧栏
    ('''<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="4" width="18" height="16" rx="3"/><line x1="9.5" y1="4" x2="9.5" y2="20"/>
          </svg>''',
     '<span class="msr ico-17">left_panel_close</span>'),
    # 2. 新对话
    ('''<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="M12 5v14M5 12h14"/>
        </svg>''',
     '<span class="msr ico-16">edit_square</span>'),
    # 3. 主题切换（图标由 applyTheme 动态同步）
    ('''<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8z"/>
          </svg>''',
     '<span class="msr ico-16" id="theme-ico">light_mode</span>'),
    # 4. 设置
    ('''<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3"/>
            <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1 1.55V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1-1.55 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.55-1H3a2 2 0 1 1 0-4h.09a1.7 1.7 0 0 0 1.55-1 1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34h.01a1.7 1.7 0 0 0 1-1.55V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1 1.55 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87v.01a1.7 1.7 0 0 0 1.55 1H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.55 1z"/>
          </svg>''',
     '<span class="msr ico-16">settings</span>'),
    # 5. 展开侧栏（汉堡）
    ('''<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <line x1="4" y1="7" x2="20" y2="7"/><line x1="4" y1="12" x2="20" y2="12"/><line x1="4" y1="17" x2="20" y2="17"/>
          </svg>''',
     '<span class="msr ico-17">menu</span>'),
    # 6. 空态 logo（闪电）
    ('''<svg viewBox="0 0 24 24" width="34" height="34" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                <path d="M13 2 4.9 12.5a.6.6 0 0 0 .47.97H11l-1 8.53L18.9 11.5a.6.6 0 0 0-.47-.97H13z"/>
              </svg>''',
     '<span class="msr es-logo-ico">bolt</span>'),
    # 7. 麦克风
    ('''<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
                <rect x="9" y="3" width="6" height="11" rx="3"/>
                <path d="M5 11a7 7 0 0 0 14 0"/>
                <line x1="12" y1="18" x2="12" y2="21"/>
              </svg>''',
     '<span class="msr ico-17">mic</span>'),
    # 8. 发送
    ('''<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                <path d="M12 19V5"/>
                <path d="M5 12l7-7 7 7"/>
              </svg>''',
     '<span class="msr ico-16">arrow_upward</span>'),
    # 9. 菜单·重命名
    ('''<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M17 3a2.8 2.8 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5z"/>
      </svg>''',
     '<span class="msr ico-14">edit</span>'),
    # 10. 菜单·删除
    ('''<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 6h18M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/>
      </svg>''',
     '<span class="msr ico-14">delete</span>'),
    # 11. 设置关闭 ✕
    ('<button class="icon-btn" id="btn-close-settings">✕</button>',
     '<button class="icon-btn" id="btn-close-settings" title="关闭">\n        <span class="msr ico-17">close</span>\n      </button>'),
])

# ---------------- main.js ----------------
patch("main.js", [
    # 1. 工具行执行中
    ("'<span class=\"t-ico\">◔</span>",
     "'<span class=\"msr t-ico\">progress_activity</span>"),
    # 2. 工具行完成/失败
    ("el.querySelector('.t-ico').textContent = ok ? '✓' : '✕';",
     "el.querySelector('.t-ico').textContent = ok ? 'check' : 'cancel';"),
    # 3. 会话更多按钮
    ("more.textContent = '⋯';",
     "more.innerHTML = '<span class=\"msr\">more_horiz</span>';"),
    # 4. 保存状态
    ("status.textContent = '✓ 已保存';",
     "status.innerHTML = '<span class=\"msr\">check</span>已保存';"),
    # 5. 记忆清除状态
    ("status.textContent = '✓ 记忆已清除';",
     "status.innerHTML = '<span class=\"msr\">check</span>记忆已清除';"),
    # 6. 欢迎语去 emoji
    ("'你好，我是 VCC ⚡\\n首次使用",
     "'你好，我是 VCC\\n首次使用"),
    # 7. 主题图标联动
    ("""  const label = document.getElementById('theme-label');
  if (label) label.textContent = dark ? '浅色模式' : '深色模式';
}""",
     """  const label = document.getElementById('theme-label');
  if (label) label.textContent = dark ? '浅色模式' : '深色模式';
  // 图标与标签同步指向「点击后进入的模式」（深色时显示太阳，浅色时显示月亮）
  const ico = document.getElementById('theme-ico');
  if (ico) ico.textContent = dark ? 'light_mode' : 'dark_mode';
}"""),
])

# ---------------- floating.js（前面 Edit 已生效，幂等跳过） ----------------
patch("floating.js", [
    ("// SF Symbols 风格步骤图标（SVG）", "// M3 步骤图标（Material Symbols Rounded，与主窗工具行同套）"),
    ("'<svg class=\"s-spin\" viewBox=\"0 0 16 16\" fill=\"none\"><circle cx=\"8\" cy=\"8\" r=\"6\" stroke=\"currentColor\" stroke-width=\"2.2\" stroke-dasharray=\"27\" stroke-dashoffset=\"9\" stroke-linecap=\"round\"/></svg>'",
     "'<span class=\"msr\">progress_activity</span>'"),
    ("'<svg viewBox=\"0 0 16 16\" fill=\"none\"><circle cx=\"8\" cy=\"8\" r=\"7\" fill=\"currentColor\" opacity=\"0.14\"/><path d=\"M4.8 8.2l2.2 2.2 4.2-4.6\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/></svg>'",
     "'<span class=\"msr\">check</span>'"),
    ("'<svg viewBox=\"0 0 16 16\" fill=\"none\"><circle cx=\"8\" cy=\"8\" r=\"7\" fill=\"currentColor\" opacity=\"0.14\"/><path d=\"M5.4 5.4l5.2 5.2M10.6 5.4l-5.2 5.2\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\"/></svg>'",
     "'<span class=\"msr\">cancel</span>'"),
])

# ---------------- floating.html / floating.css（Edit 可能已生效，幂等） ----------------
patch("floating.html", [
    ('<link rel="stylesheet" href="floating.css" />',
     '<link rel="stylesheet" href="icons.css" />\n  <link rel="stylesheet" href="floating.css" />'),
])
patch("floating.css", [
    (".f-step .s-ico svg { width: 15px; height: 15px; display: block; }",
     ".f-step .s-ico .msr { font-size: 15px; display: block; }"),
    (".f-step.running .s-spin { animation: f-spin 0.85s cubic-bezier(0.4, 0, 0.6, 1) infinite; }",
     ".f-step.running .s-ico .msr { animation: f-spin 0.85s cubic-bezier(0.4, 0, 0.6, 1) infinite; }"),
])

print("ALL PATCHES DONE")
