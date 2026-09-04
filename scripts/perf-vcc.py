# VCC 性能实测 v2：CPU（进程树）+ GPU（WebView2 引擎）双计数
# VCC_DEMO=1：墙钟 1.2s listening → 3.7s done → 4.6s idle 停绘
import psutil, subprocess, time, os, threading, re

EXE = r"D:\My things\Learn\高二\VCC\src-tauri\target\release\voice-control-for-class.exe"
NCPU = psutil.cpu_count()

subprocess.run(["taskkill", "/IM", "voice-control-for-class.exe", "/F"], capture_output=True)
time.sleep(1)

env = dict(os.environ); env["VCC_DEMO"] = "1"
p = subprocess.Popen([EXE], creationflags=0x00000008 | 0x00000200,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
time.sleep(1.4)  # 尽早进入采样（listening 已开始）

main = psutil.Process(p.pid)
gpu_lines = []
gpu_stop = threading.Event()

def gpu_sampler():
    """typeperf GPU Engine 计数器，1s 采样，抓 18 个点"""
    try:
        proc = subprocess.Popen(
            ["typeperf", r"\GPU Engine(*)\Utilization Percentage", "-si", "1", "-sc", "18"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        for line in proc.stdout:
            if gpu_stop.is_set(): proc.kill(); break
            if line.startswith('"') or line.startswith('"'):  # 数据行
                gpu_lines.append(line)
    except Exception:
        pass

th = threading.Thread(target=gpu_sampler); th.start()

tree = [main] + main.children(recursive=True)
for pr in tree:
    try: pr.cpu_percent(None)
    except psutil.NoSuchProcess: pass

samples = []
t0 = time.time()
while time.time() - t0 < 15:
    procs = [main] + main.children(recursive=True)
    cpu = 0.0; mem = 0.0
    for pr in procs:
        try:
            cpu += pr.cpu_percent(None)
            mem += pr.memory_info().rss
        except psutil.NoSuchProcess: pass
    samples.append((time.time() - t0, cpu / NCPU, mem / 1048576))
    time.sleep(1.0)

gpu_stop.set(); th.join(timeout=5)

# 解析 GPU 行：实例名 pid_NNNN，值在最后一个引号字段
vcc_pids = set()
for pr in [main] + main.children(recursive=True):
    try: vcc_pids.add(pr.pid)
    except psutil.NoSuchProcess: pass
gpu_by_t = {}
for line in gpu_lines:
    m_pid = re.search(r'pid_(\d+)', line)
    if not m_pid or int(m_pid.group(1)) not in vcc_pids: continue
    m_val = re.findall(r'"([\d\.]+)"', line)
    if m_val:
        t = len(gpu_by_t)
        gpu_by_t[t] = gpu_by_t.get(t, 0) + float(m_val[-1])

print(f"进程树 {len(vcc_pids)} 进程 | 核数 {NCPU}")
active = [s for s in samples if s[0] <= 3.2]
idle = [s for s in samples if s[0] > 4.6]
def stat(win, name):
    if not win: return f"{name}: n/a"
    cs = [s[1] for s in win]; ms = [s[2] for s in win]
    return f"{name}: CPU 均值 {sum(cs)/len(cs):.2f}% / 峰值 {max(cs):.2f}% | 内存均值 {sum(ms)/len(ms):.0f}MB"
print(stat(active, "光环活跃(0-3.2s) "))
print(stat(idle,   "待机停绘(4.6s+)  "))
print(f"GPU Utilization 采样点: {len(gpu_by_t)}")
for i, s in enumerate(samples):
    g = gpu_by_t.get(i)
    bar = '#' * int(s[1] * 4)
    print(f"  t={s[0]:4.0f}s  cpu={s[1]:5.2f}%  mem={s[2]:5.0f}MB  gpu={('%.1f%%' % g) if g is not None else '  - '}  {bar}")
