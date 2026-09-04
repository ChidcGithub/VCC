# -*- coding: utf-8 -*-
"""make-test-wav2.py — TTS 在 %TEMP% 生成（ASCII 路径），Python 重采样 16k 到 tools/tests"""
import subprocess, os, sys, wave, struct, tempfile

OUT_DIR = r"D:\My things\Learn\高二\VCC\tools\tests"
os.makedirs(OUT_DIR, exist_ok=True)
TMP = tempfile.gettempdir()

PHRASES = {
    "tts-zhi1": "把音量调到三十，然后打开课件文件夹",
    "tts-zhi2": "帮我把屏幕亮度调到五十",
    "tts-long": "帮我打开记事本，新建一个文档，写上今天的学习计划，然后把电脑音量调低一点，最后把窗口最小化",
}

lines = ["$voice = New-Object -ComObject SAPI.Speech", "$voice.Rate = 0", "$voice.Volume = 100"]
for name, text in PHRASES.items():
    src = os.path.join(TMP, f"vcc_tts_{name}.wav")
    lines.append(f"$voice.SetOutputToWaveFile('{src}')")
    lines.append(f"$voice.Speak('{text}')")
lines += ["$voice.SetOutputToNull()", "Write-Output 'TTS-DONE'"]

r = subprocess.run(["powershell", "-NoProfile", "-Command", "; ".join(lines)],
                   capture_output=True, text=True, encoding="gbk", errors="replace")
if "TTS-DONE" not in (r.stdout or ""):
    print("TTS fail:", (r.stdout or "")[:300], (r.stderr or "")[:300])
    sys.exit(1)
print("TTS raw files done")

def to16k_mono(path_in, path_out):
    with wave.open(path_in, "rb") as w:
        rate, ch, sw = w.getframerate(), w.getnchannels(), w.getsampwidth()
        raw = w.readframes(w.getnframes())
    samples = struct.unpack(f"<{len(raw)//2}h", raw) if sw == 2 else None
    if samples is None:
        raise ValueError(f"sampwidth {sw}")
    if ch > 1:
        mono = [sum(samples[i:i+ch]) // ch for i in range(0, len(samples), ch)]
    else:
        mono = list(samples)
    ratio = rate / 16000.0
    out_len = int(len(mono) / ratio)
    out = []
    for i in range(out_len):
        pos = i * ratio
        i0 = int(pos)
        frac = pos - i0
        s0 = mono[i0] if i0 < len(mono) else 0
        s1 = mono[i0+1] if i0+1 < len(mono) else 0
        v = s0 + (s1 - s0) * frac
        out.append(max(-32768, min(32767, int(v))))
    with wave.open(path_out, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes(struct.pack(f"<{len(out)}h", *out))
    return out_len / 16000.0

for name in PHRASES:
    src = os.path.join(TMP, f"vcc_tts_{name}.wav")
    dst = os.path.join(OUT_DIR, f"{name}.wav")
    dur = to16k_mono(src, dst)
    os.remove(src)
    print(f"{name}: {dur:.2f}s @16k mono")
print("ALL DONE")
