# -*- coding: utf-8 -*-
"""e2e-0500.py — v0.5.0 验证：空状态快捷指令 + 历史恢复回归 + 设置面板分组
1) 清空 history/memory → VCC_DEMO=1 启动 → 截图1：应显示 ⚡ 空状态 + chips
2) 注入历史 → 重启 → 截图2：历史气泡恢复（回归）
3) 点击 ⚙ → 截图3：设置面板应有「连接/通用/语音识别/记忆」分组标题
4) 清理
"""
import ctypes, time, subprocess, os, sys, json

u32 = ctypes.windll.user32
from PIL import ImageGrab

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    pass

DETACHED = 0x00000008 | 0x00000200
EXE = r"D:\My things\Learn\高二\VCC\src-tauri\target\release\voice-control-for-class.exe"
APPDATA_DIR = os.path.join(os.environ.get("APPDATA", r"C:\Users\chidc\AppData\Roaming"), "com.chidc.vcc")
os.makedirs(APPDATA_DIR, exist_ok=True)
HIST = os.path.join(APPDATA_DIR, "history.json")
MEM = os.path.join(APPDATA_DIR, "memory.json")
OUT = r"D:\My things\Learn\高二\VCC\ui-shots"

def kill():
    subprocess.run(["taskkill", "/IM", "voice-control-for-class.exe", "/F"], capture_output=True)
    time.sleep(1)

def launch_and_wait():
    env = dict(os.environ)
    env["VCC_DEMO"] = "1"
    p = subprocess.Popen([EXE], creationflags=DETACHED,
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                         env=env)
    hwnd = None
    for _ in range(30):
        time.sleep(0.3)
        hwnd = u32.FindWindowW(None, "Voice Control for Class")
        if hwnd and u32.IsWindow(hwnd):
            break
    if not (hwnd and u32.IsWindow(hwnd)):
        print("WINDOW NOT FOUND"); sys.exit(1)
    deadline = time.time() + 12
    while time.time() < deadline:
        if u32.IsWindowVisible(hwnd):
            break
        time.sleep(0.2)
    if not u32.IsWindowVisible(hwnd):
        print("NEVER VISIBLE"); sys.exit(1)
    time.sleep(2.2)
    return hwnd

def shot(name):
    path = os.path.join(OUT, name)
    ImageGrab.grab().save(path)
    print("saved:", path)

def crop_main(src, dst, sf=0.6):
    hwnd_hint = u32.FindWindowW(None, "Voice Control for Class")
    import ctypes.wintypes
    rect = ctypes.wintypes.RECT()
    u32.GetWindowRect(hwnd_hint, ctypes.byref(rect))
    im = ImageGrab.grab().crop((rect.left, rect.top, rect.right, rect.bottom))
    im = im.resize((int(im.width * sf), int(im.height * sf)))
    im.save(os.path.join(OUT, dst))
    print("saved:", dst)

# ---------- 1. 空状态测试 ----------
kill()
for f in (HIST, MEM):
    try: os.remove(f)
    except OSError: pass
hwnd = launch_and_wait()
shot("e2e-0500-empty-full.png")
crop_main(None, "e2e-0500-empty.png")

# ---------- 2. 历史恢复回归 ----------
kill()
test_hist = [
    {"role": "user", "content": "把音量调到 30"},
    {"role": "assistant", "content": "好的，音量已调到 30%。"},
    {"role": "user", "content": "打开 D 盘的课件文件夹"},
    {"role": "assistant", "content": "已在资源管理器中打开「课件」文件夹，里面共 6 个文件。"},
]
with open(HIST, "w", encoding="utf-8") as f:
    json.dump(test_hist, f, ensure_ascii=False)
hwnd = launch_and_wait()
shot("e2e-0500-history-full.png")
crop_main(None, "e2e-0500-history.png")

# ---------- 3. 设置面板分组 ----------
import ctypes.wintypes
rect = ctypes.wintypes.RECT()
u32.GetWindowRect(hwnd, ctypes.byref(rect))
W = rect.right - rect.left
sf = W / 412.0
x = int(rect.right - 29 * sf)
y = int(rect.top + 28 * sf)
u32.SetCursorPos(x, y)
u32.mouse_event(0x0002, 0, 0, 0, 0)
u32.mouse_event(0x0004, 0, 0, 0, 0)
time.sleep(0.8)
shot("e2e-0500-settings-full.png")
crop_main(None, "e2e-0500-settings.png")

# ---------- 4. 清理 ----------
kill()
for f in (HIST, MEM):
    try: os.remove(f)
    except OSError: pass
print("cleaned")
