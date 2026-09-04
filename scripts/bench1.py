# -*- coding: utf-8 -*-
"""bench1.py — whisper-cli 基准：模型(fp16/q5_1) x 线程(默认/6/10/14)，5s 音频
计时 = 进程端到端（含模型加载，与真实用户体验一致）
"""
import subprocess, time, os, sys

REL = r"D:\My things\Learn\高二\VCC\tools\whisper\Release"
CLI = os.path.join(REL, "whisper-cli.exe")
MODELS = {
    "fp16": r"D:\My things\Learn\高二\VCC\tools\models\ggml-small.bin",
    "q5_1": r"D:\My things\Learn\高二\VCC\tools\models\ggml-small-q5_1.bin",
}
WAV = r"D:\My things\Learn\高二\VCC\tools\tests\syn-5s.wav"

def run(model, threads=None):
    args = [CLI, "-m", model, "-f", WAV, "-l", "zh", "-nt", "-np"]
    if threads:
        args += ["-t", str(threads)]
    t0 = time.perf_counter()
    r = subprocess.run(args, capture_output=True, timeout=120,
                       creationflags=0x08000000, cwd=REL)
    dt = time.perf_counter() - t0
    return dt, r.returncode, r.stdout.decode("utf-8", "replace").strip()[:40]

print(f"{'config':<24}{'run1':>8}{'run2':>8}  out")
for mname, mpath in MODELS.items():
    for th in (None, 6, 10, 14):
        label = f"{mname} t={th or 'def'}"
        d1, rc1, out1 = run(mpath, th)
        d2, rc2, out2 = run(mpath, th)
        ok = "OK" if rc1 == 0 and rc2 == 0 else f"RC{rc1}/{rc2}"
        print(f"{label:<24}{d1:>7.2f}s{d2:>7.2f}s  {ok} {out1[:20]!r}", flush=True)
