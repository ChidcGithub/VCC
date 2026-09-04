# -*- coding: utf-8 -*-
"""e2e-0500b.py — 修正 ⚙ 坐标（窗口逻辑 357,54：卡片 inset 26 + header 内 29/28）重拍设置面板"""
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
HIST = os.path.join(APPDATA_DIR, "history.json")
OUT = r"D:\My things\Learn\高二\VCC\ui-shots"

def kill():
    subprocess.run(["taskkill", "/IM", "voice-control-for-class.exe", "/F"], capture_output=True)
    time.sleep(1)

kill()
test_hist = [
    {"role": "user", "content": "把音量调到 30"},
    {"role": "assistant", "content": "好的，音量已调到 30%。"},
]
with open(HIST, "w", encoding="utf-8") as f:
    json.dump(test_hist, f, ensure_ascii=False)

env = dict(os.environ)
env["VCC_DEMO"] = "1"
subprocess.Popen([EXE], creationflags=DETACHED,
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
time.sleep(2.2)

import ctypes.wintypes
rect = ctypes.wintypes.RECT()
u32.GetWindowRect(hwnd, ctypes.byref(rect))
W = rect.right - rect.left
sf = W / 412.0
# 窗口逻辑坐标 (357, 54)：卡片 inset 26px + header 右 16px + 按钮半宽 13px / 顶 15px + 半高 13px
x = int(rect.left + 357 * sf)
y = int(rect.top + 54 * sf)
u32.SetCursorPos(x, y)
u32.mouse_event(0x0002, 0, 0, 0, 0)
u32.mouse_event(0x0004, 0, 0, 0, 0)
time.sleep(0.9)
ImageGrab.grab().save(os.path.join(OUT, "e2e-0500-settings-full.png"))
im = ImageGrab.grab().crop((rect.left, rect.top, rect.right, rect.bottom))
im = im.resize((int(im.width * 0.6), int(im.height * 0.6)))
im.save(os.path.join(OUT, "e2e-0500-settings.png"))
print("saved, sf=", round(sf, 3), "click=", x, y)
kill()
try: os.remove(HIST)
except OSError: pass
print("cleaned")
