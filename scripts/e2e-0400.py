# -*- coding: utf-8 -*-
"""e2e-0400.py — v0.4.0 UI 回归：历史恢复 + 设置面板模型下拉"""
import ctypes, time, subprocess, os, json
from PIL import ImageGrab

u32 = ctypes.windll.user32
import ctypes.wintypes
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    pass

DETACHED = 0x00000008 | 0x00000200
EXE = r"D:\My things\Learn\高二\VCC\src-tauri\target\release\voice-control-for-class.exe"
APPDATA_DIR = os.path.join(os.environ.get("APPDATA", r"C:\Users\chidc\AppData\Roaming"), "com.chidc.vcc")
HIST = os.path.join(APPDATA_DIR, "history.json")
MEM = os.path.join(APPDATA_DIR, "memory.json")
OUT = r"D:\My things\Learn\高二\VCC\ui-shots"

def grab(p):
    ImageGrab.grab().save(p)

def kill():
    subprocess.run(["taskkill", "/IM", "voice-control-for-class.exe", "/F"], capture_output=True)
    subprocess.run(["taskkill", "/IM", "whisper-server.exe", "/F"], capture_output=True)
    time.sleep(1)

kill()
with open(HIST, "w", encoding="utf-8") as f:
    json.dump([
        {"role": "user", "content": "把音量调到 30"},
        {"role": "assistant", "content": "好的，音量已调到 30%。"},
    ], f, ensure_ascii=False)
with open(MEM, "w", encoding="utf-8") as f:
    json.dump({"summary": "回归测试记忆。", "updated_at": "x"}, f, ensure_ascii=False)

env = dict(os.environ)
env["VCC_DEMO"] = "1"
p = subprocess.Popen([EXE], creationflags=DETACHED,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
hwnd = None
for _ in range(30):
    time.sleep(0.3)
    hwnd = u32.FindWindowW(None, "Voice Control for Class")
    if hwnd and u32.IsWindow(hwnd):
        break
deadline = time.time() + 12
while time.time() < deadline:
    if u32.IsWindowVisible(hwnd):
        break
    time.sleep(0.2)
time.sleep(2.0)

rect = ctypes.wintypes.RECT()
u32.GetWindowRect(hwnd, ctypes.byref(rect))
grab(os.path.join(OUT, "e2e-0400-history.png"))

# 点 ⚙（窗口逻辑坐标 357, 54，卡片 inset 26 已计入）
sf = (rect.right - rect.left) / 412.0
x = int(rect.left + 357 * sf)
y = int(rect.top + 54 * sf)
u32.SetCursorPos(x, y)
u32.mouse_event(0x0002, 0, 0, 0, 0)
u32.mouse_event(0x0004, 0, 0, 0, 0)
time.sleep(0.8)
grab(os.path.join(OUT, "e2e-0400-settings.png"))

kill()
for f in (HIST, MEM):
    try: os.remove(f)
    except OSError: pass

# 裁剪输出
im = ImageGrab.grab.__self__ if False else None
from PIL import Image
for name in ("e2e-0400-history", "e2e-0400-settings"):
    im = Image.open(os.path.join(OUT, name + ".png"))
    crop = im.crop((rect.left, rect.top, rect.right, rect.bottom))
    crop = crop.resize((int(crop.width * 0.55), int(crop.height * 0.55)))
    crop.save(os.path.join(OUT, name + "-c.png"))
print("done")
