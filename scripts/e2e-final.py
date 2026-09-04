# VCC E2E v5：VCC_DEMO=1 内置钩子 → 全链路真实触发 → 截屏
import ctypes, time, subprocess, os, sys

u32 = ctypes.windll.user32
from PIL import ImageGrab

DETACHED_PROCESS = 0x00000008
CREATE_NEW_PROCESS_GROUP = 0x00000200
EXE = r"D:\My things\Learn\高二\VCC\src-tauri\target\release\voice-control-for-class.exe"
SHOT = rf"D:\My things\Learn\高二\VCC\ui-shots\{sys.argv[1] if len(sys.argv) > 1 else 'e2e-v5'}.png"

subprocess.run(["taskkill", "/IM", "voice-control-for-class.exe", "/F"], capture_output=True)
time.sleep(1)

env = dict(os.environ)
env["VCC_DEMO"] = "1"
p = subprocess.Popen([EXE], creationflags=DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     env=env)
print("pid:", p.pid)

hwnd = None
for _ in range(30):
    time.sleep(0.3)
    hwnd = u32.FindWindowW(None, "Voice Control for Class")
    if hwnd and u32.IsWindow(hwnd):
        break

# 等内置钩子（启动后 1.2s）触发 + 光环淡入
deadline = time.time() + 12
visible_at = None
while time.time() < deadline:
    if hwnd and u32.IsWindowVisible(hwnd):
        visible_at = time.time()
        break
    time.sleep(0.2)

if visible_at:
    print(f"main VISIBLE (demo hook fired)")
    time.sleep(1.6)  # listening 光环满强度 + 悬浮窗 running 态
    ImageGrab.grab().save(SHOT)
    print("saved:", SHOT)
    # 第二张：hook 3.7s 发 done，900ms 窗口内抓绽放峰值（光环 done + 悬浮窗全勾 + 输入条扫光加速）
    base = SHOT.rsplit(".", 1)[0]
    time.sleep(1.1)
    ImageGrab.grab().save(base + "-done.png")
    print("saved:", base + "-done.png")
    # 第三张：done 后 5s 淡出 + 0.45s hide → 悬浮窗应已消失（生命周期闭环验证）
    time.sleep(5.3)
    ImageGrab.grab().save(base + "-fade.png")
    print("saved:", base + "-fade.png")
else:
    print("main NEVER visible")
    sys.exit(1)
