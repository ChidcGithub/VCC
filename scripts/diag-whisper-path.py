# -*- coding: utf-8 -*-
"""诊断 whisper 对非 ASCII 绝对路径的崩溃：空格 vs 中文 vs 修复方案"""
import subprocess, ctypes, os, sys, time, urllib.request

REL = r"D:\My things\Learn\高二\VCC\tools\whisper\Release"
CLI = os.path.join(REL, "whisper-cli.exe")
SRV = os.path.join(REL, "whisper-server.exe")
MODELS_DIR = r"D:\My things\Learn\高二\VCC\tools\models"
MODEL = os.path.join(MODELS_DIR, "ggml-small-q5_1.bin")
MNAME = "ggml-small-q5_1.bin"
WAV = r"C:\Users\chidc\AppData\Local\Temp\vcc_rec_test.wav"

k32 = ctypes.windll.kernel32
print(f"GetACP={k32.GetACP()} GetOEMCP={k32.GetOEMCP()}")

SP_DIR = r"D:\vcc space test"
CN_DIR = r"D:\高二测试"
os.makedirs(SP_DIR, exist_ok=True)
os.makedirs(CN_DIR, exist_ok=True)
M_SP = os.path.join(SP_DIR, MNAME)
M_CN = os.path.join(CN_DIR, MNAME)
for dst in (M_SP, M_CN):
    if not os.path.exists(dst):
        os.link(MODEL, dst)

buf = ctypes.create_unicode_buffer(1024)
n = k32.GetShortPathNameW(MODEL, buf, 1024)
SHORT = buf.value if n else None
print(f"short_path={SHORT}")

def run_cli(model, label, cwd, extra=None):
    args = [CLI, "-m", model, "-f", WAV, "-l", "zh", "-t", "4", "-np"]
    if extra:
        args += extra
    try:
        p = subprocess.run(args, capture_output=True, timeout=120, cwd=cwd)
        rc = p.returncode
    except subprocess.TimeoutExpired:
        print(f"[CLI {label}] TIMEOUT")
        return
    err = p.stderr.decode("utf-8", "replace").strip().splitlines()
    out = p.stdout.decode("utf-8", "replace").strip().splitlines()
    print(f"[CLI {label}] rc={rc} (0x{rc & 0xFFFFFFFF:08X}) stderr={len(err)}行 stdout={len(out)}行")
    for line in err[-2:]:
        print("   err|", line[:150])
    for line in out[-1:]:
        print("   out|", line[:150])
    sys.stdout.flush()

print("\n===== CLI 矩阵 =====")
run_cli(MODEL, "abs-real(对照-预期死)", REL)
run_cli(M_SP, "纯空格无中文", REL)
run_cli(M_CN, "纯中文无空格", REL)
if SHORT:
    run_cli(SHORT, "8.3短路径", REL)
run_cli(MNAME, "修复候选:cwd=models+相对名", MODELS_DIR)

def run_srv(model, label, cwd, extra, probe=True):
    args = [SRV, "-m", model, "--host", "127.0.0.1", "--port", "18779"] + extra
    p = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, cwd=cwd)
    time.sleep(6)
    rc = p.poll()
    print(f"[SRV {label}] alive_after_6s={rc is None} rc={rc}")
    if rc is not None:
        out = p.stdout.read().decode("utf-8", "replace")
        print("   tail|", out[-500:].replace("\n", "\n        "))
        sys.stdout.flush()
        return
    if probe:
        try:
            boundary = "----vccdiag"
            data = open(WAV, "rb").read()
            body = (f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\n"
                    f"Content-Type: audio/wav\r\n\r\n").encode() + data + f"\r\n--{boundary}--\r\n".encode()
            req = urllib.request.Request("http://127.0.0.1:18779/inference", data=body,
                                         headers={"Content-Type": f"multipart/form-data; boundary={boundary}"})
            r = urllib.request.urlopen(req, timeout=90)
            print("   推理结果:", r.read().decode("utf-8", "replace")[:200])
        except Exception as e:
            print("   推理失败:", repr(e)[:200])
    p.terminate()
    try:
        p.wait(5)
    except Exception:
        p.kill()
    sys.stdout.flush()

print("\n===== SERVER 矩阵 =====")
APP_ARGS = ["-nt", "-l", "zh", "-bs", "1", "-bo", "1", "--prompt", "以下是普通话的句子。", "-t", "4"]
run_srv(MODEL, "abs+App参数(对照)", REL, APP_ARGS, probe=False)
run_srv(MNAME, "修复候选:cwd=models+相对名(无prompt)", MODELS_DIR, ["-nt", "-l", "zh", "-bs", "1", "-bo", "1", "-t", "4"])
print("\ndone")
