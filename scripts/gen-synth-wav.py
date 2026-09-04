# -*- coding: utf-8 -*-
"""gen-synth-wav.py — 合成"语音样"测试音频（速度基准用；whisper 耗时只与时长相关）
带基频谐波 + 4Hz 词节奏包络 + 词间静音 + 轻噪，16k mono
"""
import wave, struct, math, os, random

OUT_DIR = r"D:\My things\Learn\高二\VCC\tools\tests"
os.makedirs(OUT_DIR, exist_ok=True)
SR = 16000

def synth(duration_s, path, seed=42):
    rnd = random.Random(seed)
    n = int(SR * duration_s)
    frames = []
    # 语音节奏：约 3.5 词/秒，词长 0.15-0.35s，词间静音 0.05-0.15s
    t = 0.0
    segments = []  # (start, end)
    while t < duration_s:
        wl = rnd.uniform(0.15, 0.35)
        segments.append((t, min(t + wl, duration_s)))
        t += wl + rnd.uniform(0.05, 0.15)
    f0 = rnd.uniform(120, 220)
    for i in range(n):
        ts = i / SR
        voiced = any(a <= ts < b for a, b in segments)
        if voiced:
            # 基频 + 谐波 + 音高微移
            f = f0 * (1.0 + 0.06 * math.sin(2 * math.pi * 1.3 * ts))
            v = sum(math.sin(2 * math.pi * f * k * ts) / k for k in (1, 2, 3, 4, 5))
            env = 0.55 + 0.45 * math.sin(2 * math.pi * 4.0 * ts)
            s = v * env * 0.35
        else:
            s = 0.0
        s += rnd.uniform(-0.01, 0.01)  # 轻噪
        frames.append(max(-32768, min(32767, int(s * 32767))))
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(struct.pack(f"<{len(frames)}h", *frames))
    print(f"{os.path.basename(path)}: {duration_s}s")

synth(3.0, os.path.join(OUT_DIR, "syn-3s.wav"))
synth(5.0, os.path.join(OUT_DIR, "syn-5s.wav"), seed=7)
synth(10.0, os.path.join(OUT_DIR, "syn-10s.wav"), seed=13)
print("DONE")
