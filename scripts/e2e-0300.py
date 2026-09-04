# -*- coding: utf-8 -*-
"""e2e-0300.py — v0.3.0 验证：历史恢复渲染 + 设置面板记忆区块 + 圆角
1) 注入测试 history.json / memory.json
2) VCC_DEMO=1 启动（自动呼出主窗）
3) 截图1：对话流应出现「上次对话」分隔线 + 历史 user/assistant 气泡
4) 点击 ⚙ 打开设置 → 截图2：AI 长期记忆 textarea 应显示注入的记忆
5) 清理：杀进程 + 删测试数据
"""
import ctypes, time, subprocess, os, sys, json

u32 = ctypes.windll.user32
from PIL import ImageGrab

# DPI aware：保证 GetWindowRect 是物理像素
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

# ---------- 0. 清场 + 注入测试数据 ----------
kill()
test_hist = [
    {"role": "user", "content": "把音量调到 30"},
    {"role": "assistant", "content": "好的，音量已调到 30%。"},
    {"role": "user", "content": "打开 D 盘的课件文件夹"},
    {"role": "assistant", "content": "已在资源管理器中打开「课件」文件夹，里面共 6 个文件，需要我演示其中某个吗？"},
]
with open(HIST, "w", encoding="utf-8") as f:
    json.dump(test_hist, f, ensure_ascii=False)
test_mem = {"summary": "用户是高二学生，常用 D 盘的课件文件夹；偏好音量 30%；上课时喜欢窗口置顶。", "updated_at": "2026年9月4日 13:50"}
with open(MEM, "w", encoding="utf-8") as f:
    json.dump(test_mem, f, ensure_ascii=False)
print("test data injected")

# ---------- 1. 启动 ----------
env = dict(os.environ)
env["VCC_DEMO"] = "1"
p = subprocess.Popen([EXE], creationflags=DETACHED,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     env=env)
print("pid:", p.pid)

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

time.sleep(2.2)  # 历史渲染 + 入场动画结束

# ---------- 2. 截图1：历史恢复 ----------
shot1 = os.path.join(OUT, "e2e-0300-history.png")
ImageGrab.grab().save(shot1)
print("saved:", shot1)

# ---------- 3. 点击 ⚙ 打开设置 ----------
rect = ctypes.wintypes.RECT() if hasattr(ctypes, "wintypes") else None
import ctypes.wintypes
rect = ctypes.wintypes.RECT()
u32.GetWindowRect(hwnd, ctypes.byref(rect))
W = rect.right - rect.left
sf = W / 412.0  # 逻辑宽 412
# btn-settings 中心：右缘内 16+13=29 逻辑 px，顶 15+13=28 逻辑 px
x = int(rect.right - 29 * sf)
y = int(rect.top + 28 * sf)
u32.SetCursorPos(x, y)
u32.mouse_event(0x0002, 0, 0, 0, 0)  # LEFTDOWN
u32.mouse_event(0x0004, 0, 0, 0, 0)  # LEFTUP
time.sleep(0.8)

shot2 = os.path.join(OUT, "e2e-0300-settings.png")
ImageGrab.grab().save(shot2)
print("saved:", shot2)
print("window rect:", rect.left, rect.top, rect.right, rect.bottom, "sf=", round(sf, 3))

# ---------- 4. 清理 ----------
kill()
for f in (HIST, MEM):
    try: os.remove(f)
    except OSError: pass
print("cleaned")
