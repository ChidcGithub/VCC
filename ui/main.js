/* ============ VCC 主窗口逻辑 ============ */
/* 全局错误兜底：任何未捕获异常进对话流（err 气泡），不静默白屏 */
window.addEventListener('error', (e) => {
  try {
    if (e && e.message && !String(e.message).includes('ResizeObserver')) {
      addErrorBubble('内部错误：' + e.message, { noRetry: true });
    }
  } catch (_) { /* 兜底自身不能再抛 */ }
});

const TAURI = window.__TAURI__;
const invoke = TAURI ? TAURI.core.invoke : (async () => ({}));
const listen = TAURI ? TAURI.event.listen : (async () => {});

const chatEl = document.getElementById('chat');
const inputEl = document.getElementById('input');
const phaseEl = document.getElementById('phase-line');
const transcriptEl = document.getElementById('live-transcript');
const micBtn = document.getElementById('btn-mic');
const sendBtn = document.getElementById('btn-send');

const PHASE_TEXT = {
  idle: '',
  summoned: '聆听中…',   // 呼出待命（窗口已开、麦克风未开）
  listening: '聆听中…',  // 麦克风真录音中
  thinking: '思考中…',
  executing: '执行中…',
  done: '',
};

/* 跑马灯显示集：仅真录音 + 工具执行（summoned/thinking/done 不全屏亮灯） */

const emit = TAURI ? TAURI.event.emit : (async () => {});
let currentPhase = '';
let doneTimer = null;

function setPhase(p) {
  if (currentPhase === p) return;
  currentPhase = p;
  document.body.classList.remove('idle', 'summoned', 'listening', 'thinking', 'executing', 'done');
  document.body.classList.add(p);
  phaseEl.textContent = PHASE_TEXT[p] || '';
  phaseEl.classList.toggle('empty', p === 'idle');
  // done 绽放统一在此回落（Rust 端也发 done，回落逻辑只写一处）
  clearTimeout(doneTimer);
  if (p === 'done') {
    doneTimer = setTimeout(() => { if (currentPhase === 'done') setPhase('idle'); }, 900);
  }
  // 广播给全屏 overlay 跑马灯（Rust 端也监听此事件负责窗口显示/隐藏）
  if (TAURI) emit('vcc://phase', { phase: p });
}

/* 完成绽放：回答收尾时光环短暂爆亮（done），900ms 后由 setPhase 统一回 idle */
function finishGlow() {
  if (['listening', 'thinking', 'executing'].includes(currentPhase)) {
    setPhase('done');
  }
}

/* ---------- 对话渲染 ---------- */
/* 智能滚动：仅当本来就贴近底部时跟随新消息（回看历史不被强拉走） */
function scrollChat(force) {
  const nearBottom = chatEl.scrollHeight - chatEl.scrollTop - chatEl.clientHeight < 90;
  if (force || nearBottom) chatEl.scrollTop = chatEl.scrollHeight;
}

function nowHM() {
  const d = new Date();
  return String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
}

/* AI 回答轻量 markdown：**bold** / `code`。先整体 HTML 转义再替换，杜绝注入 */
function renderInlineMd(s) {
  const esc = s
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  return esc
    .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
    .replace(/`([^`]+)`/g, '<code>$1</code>');
}

function addBubble(role, text) {
  const div = document.createElement('div');
  div.className = 'bubble ' + role;
  if (role === 'ai') div.innerHTML = renderInlineMd(text);
  else div.textContent = text;
  div.title = nowHM();
  chatEl.appendChild(div);
  scrollChat(role === 'user');
  return div;
}

/* 错误气泡 + 一键重试（重发上一条指令） */
let lastUserText = '';

/* ---------- 空态快捷指令（支持设置面板自定义） ---------- */
const DEFAULT_CMDS = ['打开记事本', '音量调到 50', '打开计算器', '一键静音'];
let currentCmds = DEFAULT_CMDS;

function renderChips(es, cmds) {
  const wrap = es.querySelector('.es-chips');
  if (!wrap) return;
  wrap.innerHTML = '';
  for (const c of cmds) {
    const b = document.createElement('button');
    b.className = 'chip';
    b.textContent = c;
    b.addEventListener('click', () => send(c));
    wrap.appendChild(b);
  }
}

function addErrorBubble(msg, opts = {}) {
  const div = document.createElement('div');
  div.className = 'bubble err';
  const span = document.createElement('span');
  span.className = 'err-text';
  span.textContent = msg;
  div.appendChild(span);
  if (!opts.noRetry) {
    const btn = document.createElement('button');
    btn.className = 'retry-chip';
    btn.textContent = opts.settings ? '打开设置' : '重试';
    btn.addEventListener('click', () => {
      div.remove();
      if (opts.settings) document.getElementById('btn-settings').click();
      else if (lastUserText) send(lastUserText);
    });
    div.appendChild(btn);
  }
  chatEl.appendChild(div);
  scrollChat(true); // 错误必须被看见
}

function hideEmptyState() {
  const es = document.getElementById('empty-state');
  if (es) es.remove();
}

/* ---------- TTS 朗读（WebView2 原生 speechSynthesis，零依赖） ---------- */
let ttsOn = false;

function speak(text) {
  if (!ttsOn || !('speechSynthesis' in window) || !text || !text.trim()) return;
  try {
    speechSynthesis.cancel(); // 打断上一条，防叠音
    const u = new SpeechSynthesisUtterance(text);
    u.lang = 'zh-CN';
    u.rate = 1.05;
    speechSynthesis.speak(u);
  } catch (_) { /* TTS 失败静默 */ }
}

function stopSpeak() {
  try { if ('speechSynthesis' in window) speechSynthesis.cancel(); } catch (_) {}
}

function addDivider(text) {
  const div = document.createElement('div');
  div.className = 'chat-divider';
  div.textContent = text;
  chatEl.appendChild(div);
  scrollChat(true);
}

function addToolLine(label) {
  const div = document.createElement('div');
  div.className = 'tool-line running';
  div.innerHTML = '<span class="t-ico">◔</span><span class="t-label"></span><span class="t-ms"></span>';
  div.querySelector('.t-label').textContent = label;
  div.title = nowHM();
  chatEl.appendChild(div);
  scrollChat(true);
  return div;
}

function finishToolLine(el, ok, ms) {
  if (!el) return;
  el.classList.remove('running');
  el.classList.add(ok ? 'done' : 'fail');
  el.querySelector('.t-ico').textContent = ok ? '✓' : '✕';
  if (ms != null) {
    let m = el.querySelector('.t-ms');
    if (m) m.textContent = ms >= 1000 ? (ms / 1000).toFixed(1) + 's' : Math.round(ms) + 'ms';
  }
}

/* ---------- 发送 ---------- */
let agentBusy = false;
/* 输入历史：↑ 调出上一条指令，↓ 返回；未进入历史时暂存草稿 */
let sendBusyTimer = null;
const inputHist = [];
let histPos = -1;
let histDraft = '';

function histNav(dir) { // -1 = 更早(↑)，+1 = 更新(↓)
  if (!inputHist.length) return;
  if (histPos === -1) {
    if (dir === 1) return;
    histDraft = inputEl.value;
    histPos = inputHist.length - 1;
  } else {
    histPos += dir;
    if (histPos >= inputHist.length) { histPos = -1; inputEl.value = histDraft; return; }
    if (histPos < 0) histPos = 0;
  }
  inputEl.value = inputHist[histPos];
}

async function send(text) {
  text = (text || '').trim();
  if (!text) return;
  if (agentBusy) {
    // 静默吞输入会让人困惑：状态行轻提示 1.5s，输入内容保留
    phaseEl.textContent = '上一条还在执行中…';
    phaseEl.classList.remove('empty');
    clearTimeout(sendBusyTimer);
    sendBusyTimer = setTimeout(() => {
      if (!agentBusy) {
        phaseEl.textContent = '';
        phaseEl.classList.add('empty');
      }
    }, 1500);
    return;
  }
  lastUserText = text;
  stopSpeak();
  hideEmptyState();
  inputEl.value = '';
  // 输入历史（↑/↓ 翻阅；相邻去重）
  if (text !== inputHist[inputHist.length - 1]) inputHist.push(text);
  if (inputHist.length > 50) inputHist.shift();
  histPos = -1;
  addBubble('user', text);
  agentBusy = true;
  sendBtn.disabled = true;
  setPhase('thinking');
  try {
    await invoke('agent_run', { text });
  } catch (e) {
    setPhase('idle');
    const msg = String(e);
    const keyErr = msg.includes('401') || msg.includes('403') || msg.includes('未配置 API Key');
    const hint = msg.includes('未配置 API Key')
      ? '还没配置 API Key，点「打开设置」填入即可开始使用。'
      : keyErr
        ? 'API Key 无效或已过期。'
        : 'Agent 调用失败：' + e;
    addErrorBubble(hint, { settings: keyErr });
  } finally {
    agentBusy = false;
    sendBtn.disabled = false;
    // 兜底：流结束事件未触发绽放时（如纯工具轮次），在这里收尾
    finishGlow();
  }
}

sendBtn.addEventListener('click', () => send(inputEl.value));
inputEl.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.isComposing) send(inputEl.value);
  else if (e.key === 'ArrowUp' && !e.isComposing) { histNav(-1); e.preventDefault(); }
  else if (e.key === 'ArrowDown' && !e.isComposing) { histNav(1); e.preventDefault(); }
});

/* ESC 快速收起主窗（课堂场景一键隐藏；热键或托盘可再呼出）。
   设置面板打开时先关面板。 */
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    const s = document.getElementById('settings');
    if (s && !s.classList.contains('hidden')) s.classList.add('hidden');
    else invoke('hide_main');
  } else if (e.ctrlKey && !e.shiftKey && !e.altKey && (e.key === 'n' || e.key === 'N')) {
    // Ctrl+N：新对话（键盘流，手不离键盘）
    e.preventDefault();
    document.getElementById('btn-new').click();
  }
});

/* ---------- 事件（Rust → 前端） ---------- */
let runningTools = {};
const toolStarts = {};
let streamBubble = null;

listen('vcc://phase', (e) => setPhase(e.payload.phase));

listen('vcc://chat', (e) => {
  const { role, text } = e.payload;
  addBubble(role, text);
  if (role === 'ai') {
    setPhase('idle');
    speak(text);
  }
});

// 流式回答（打字机）
listen('vcc://chat-start', () => {
  streamBubble = addBubble('ai', '');
});
listen('vcc://chat-delta', (e) => {
  if (!streamBubble) streamBubble = addBubble('ai', '');
  streamBubble.textContent += e.payload.text;
  scrollChat(false);
});
listen('vcc://chat-end', () => {
  if (streamBubble) {
    streamBubble.innerHTML = renderInlineMd(streamBubble.textContent);
    speak(streamBubble.textContent);
  }
  streamBubble = null;
  if (!document.body.classList.contains('executing')) finishGlow();
});

listen('vcc://tool', (e) => {
  const { id, label } = e.payload;
  toolStarts[id] = performance.now();
  runningTools[id] = addToolLine(label);
  if (document.body.classList.contains('thinking')) setPhase('executing');
});

listen('vcc://tool-done', (e) => {
  const { id, ok } = e.payload;
  const ms = toolStarts[id] != null ? performance.now() - toolStarts[id] : null;
  finishToolLine(runningTools[id], ok, ms);
  delete toolStarts[id];
  delete runningTools[id];
});

listen('vcc://invoked', () => {
  // Agent 执行中呼出不覆盖 phase（thinking/executing 是真实进行中的状态）
  if (!agentBusy) setPhase('summoned');
  inputEl.focus();
  // 呼出入场：卡片 Apple 式弹入（一次性动画，重触发用 reflow 重启）
  document.body.classList.remove('summon');
  void document.body.offsetWidth;
  document.body.classList.add('summon');
});

listen('vcc://transcript', (e) => {
  showTranscript(e.payload.text);
});

/* AI 长期记忆后台更新完成（上下文压缩 / 新对话归档时触发） */
listen('vcc://memory-updated', () => {
  addDivider('记忆已更新');
});

/* ---------- 悬浮识别文字 ---------- */
let transcriptTimer = null;
function showTranscript(text) {
  transcriptEl.textContent = text;
  transcriptEl.classList.remove('hidden');
  // 重新触发入场动画
  transcriptEl.style.animation = 'none';
  void transcriptEl.offsetWidth;
  transcriptEl.style.animation = '';
  clearTimeout(transcriptTimer);
  transcriptTimer = setTimeout(hideTranscript, 6000);
}
function hideTranscript() {
  transcriptEl.classList.add('hidden');
}

/* ---------- 录音（按住说话）---------- */
let recorder = null;

/* 识别完成提示音：教师背对学生时靠听觉确认（正弦短音 + 快速衰减，音量克制） */
let fx = null;
function ding(freq = 880, dur = 0.09, gain = 0.045) {
  try {
    fx = fx || new AudioContext();
    if (fx.state === 'suspended') fx.resume();
    const o = fx.createOscillator();
    const g = fx.createGain();
    o.type = 'sine';
    o.frequency.value = freq;
    g.gain.setValueAtTime(gain, fx.currentTime);
    g.gain.exponentialRampToValueAtTime(0.0001, fx.currentTime + dur);
    o.connect(g);
    g.connect(fx.destination);
    o.start();
    o.stop(fx.currentTime + dur);
  } catch (_) { /* 音频失败静默 */ }
}

async function startRecording() {
  if (recorder) return; // 防双击/重复 pointerdown 泄漏麦克风流
  try {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    });
    const ctx = new AudioContext();
    const src = ctx.createMediaStreamSource(stream);
    // Whisper 需要 16kHz 单声道 Int16
    const proc = ctx.createScriptProcessor(4096, 1, 1);
    const chunks = [];
    proc.onaudioprocess = (ev) => {
      chunks.push(new Float32Array(ev.inputBuffer.getChannelData(0)));
    };
    src.connect(proc);
    proc.connect(ctx.destination);

    recorder = { ctx, stream, proc, src, chunks, startedAt: Date.now() };
    document.body.classList.add('recording');
    // 记录录音前状态（summoned/idle/thinking…），松手后未被 send 接管时恢复它
    recorder.prevPhase = currentPhase === 'listening' ? 'idle' : currentPhase;
    // 真录音 → listening：跑马灯此刻才亮（呼出窗口 ≠ 开麦）
    setPhase('listening');
    // 60s 上限：超长录音会拖垮低配设备的识别推理，到时自动停止
    recorder.maxTimer = setTimeout(() => { if (recorder) stopRecording(); }, 60000);

    // 语音响应：RMS 电平 → 20Hz 节流事件 → overlay 光环随声音呼吸
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 512;
    src.connect(analyser);
    recorder.analyser = analyser;
    const tdata = new Uint8Array(analyser.fftSize);
    let lastEmit = 0;
    const pump = () => {
      if (!recorder || recorder.analyser !== analyser) return;
      analyser.getByteTimeDomainData(tdata);
      let sum = 0;
      for (let i = 0; i < tdata.length; i++) {
        const v = (tdata[i] - 128) / 128;
        sum += v * v;
      }
      const rms = Math.sqrt(sum / tdata.length);
      const level = Math.min(1, rms * 5.0);
      const now = performance.now();
      if (now - lastEmit > 50) {
        lastEmit = now;
        emit('vcc://level', { level });
      }
      recorder.levelRaf = requestAnimationFrame(pump);
    };
    pump();
  } catch (e) {
    addBubble('err', '无法访问麦克风：' + e);
  }
}

async function stopRecording() {
  if (!recorder) return;
  const r = recorder;
  recorder = null;
  document.body.classList.remove('recording');
  clearTimeout(r.maxTimer || 0);
  cancelAnimationFrame(r.levelRaf || 0);
  emit('vcc://level', { level: 0 });
  r.proc.disconnect();
  r.src.disconnect();
  r.stream.getTracks().forEach((t) => t.stop());
  r.ctx.close();

  // 未被 send() 接管时的回落态：回到录音前状态（误触/没听清时「聆听中」提示保留）
  const backPhase = (!r.prevPhase || r.prevPhase === 'listening' || r.prevPhase === 'done')
    ? 'idle' : r.prevPhase;

  const durMs = Date.now() - r.startedAt;
  if (durMs < 400) { setPhase(backPhase); return; } // 太短，误触

  // 合并 + 16k 重采样 + Int16
  const total = r.chunks.reduce((n, c) => n + c.length, 0);
  if (total === 0) { setPhase(backPhase); return; }
  const merged = new Float32Array(total);
  let off = 0;
  for (const c of r.chunks) { merged.set(c, off); off += c.length; }
  const resampled = resampleTo16k(merged, r.ctx.sampleRate);
  const trimmed = trimSilence(resampled, 16000);
  const wavB64 = encodeWavBase64(trimmed, 16000);

  setPhase('thinking');
  showTranscript('识别中…');
  try {
    const text = await invoke('transcribe', { wavBase64: wavB64 });
    if (text && text.trim()) {
      ding();
      showTranscript(text.trim());
      await send(text.trim());
    } else {
      // 纯噪音/太轻：明确提示而非无声消失
      showTranscript('没听清，请靠近一点再试');
      setPhase(backPhase);
      clearTimeout(transcriptTimer);
      transcriptTimer = setTimeout(hideTranscript, 2200);
    }
  } catch (e) {
    hideTranscript();
    setPhase(backPhase);
    // 识别失败重试需要用户再按一次麦克风，误导性的「重试」按钮不如直接引导
    addErrorBubble('语音识别失败：' + e + '（请按住麦克风再说一次）', { noRetry: true });
  }
}

micBtn.addEventListener('pointerdown', (e) => {
  e.preventDefault();
  startRecording();
});
micBtn.addEventListener('pointerup', () => stopRecording());
micBtn.addEventListener('pointerleave', () => { if (recorder) stopRecording(); });

/* 静音裁剪：砍掉首尾静音段再送识别。
   whisper 推理时长与音频长度成正比，短指令录音常见 1-3s 静音，白烧推理时间 */
function trimSilence(f32, rate) {
  const win = Math.floor(rate * 0.025); // 25ms 分析窗
  const threshold = 0.012;              // RMS 门限（约 -38dB，麦克风底噪以下）
  const pad = Math.floor(rate * 0.15);  // 两侧各留 150ms 缓冲
  const frames = Math.floor(f32.length / win);
  let first = -1, last = -1;
  for (let i = 0; i < frames; i++) {
    let sum = 0;
    const base = i * win;
    for (let j = 0; j < win; j++) {
      const v = f32[base + j];
      sum += v * v;
    }
    if (Math.sqrt(sum / win) > threshold) {
      if (first === -1) first = i;
      last = i;
    }
  }
  if (first === -1) return f32; // 全静音：原样返回，由识别空文本自然兜底
  const start = Math.max(0, first * win - pad);
  const end = Math.min(f32.length, (last + 1) * win + pad);
  return f32.slice(start, end);
}

function resampleTo16k(f32, fromRate) {
  if (fromRate === 16000) return f32;
  const ratio = fromRate / 16000;
  const outLen = Math.floor(f32.length / ratio);
  const out = new Float32Array(outLen);
  for (let i = 0; i < outLen; i++) {
    const pos = i * ratio;
    const i0 = Math.floor(pos);
    const frac = pos - i0;
    const s0 = f32[i0] || 0;
    const s1 = f32[Math.min(i0 + 1, f32.length - 1)] || 0;
    out[i] = s0 + (s1 - s0) * frac;
  }
  return out;
}

function encodeWavBase64(f32, sampleRate) {
  const n = f32.length;
  const buf = new ArrayBuffer(44 + n * 2);
  const view = new DataView(buf);
  const wstr = (o, s) => { for (let i = 0; i < s.length; i++) view.setUint8(o + i, s.charCodeAt(i)); };
  wstr(0, 'RIFF'); view.setUint32(4, 36 + n * 2, true); wstr(8, 'WAVE');
  wstr(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true);
  view.setUint16(22, 1, true); view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true); view.setUint16(32, 2, true);
  view.setUint16(34, 16, true); wstr(36, 'data'); view.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++) {
    const s = Math.max(-1, Math.min(1, f32[i]));
    view.setInt16(44 + i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true);
  }
  const bytes = new Uint8Array(buf);
  let bin = '';
  const CH = 0x8000;
  for (let i = 0; i < bytes.length; i += CH) {
    bin += String.fromCharCode.apply(null, bytes.subarray(i, i + CH));
  }
  return btoa(bin);
}

/* ---------- 头部按钮 ---------- */
document.getElementById('btn-new').addEventListener('click', async () => {
  if (agentBusy) return; // 执行中不允许清上下文
  if (chatEl.dataset.clearing) return;
  chatEl.dataset.clearing = '1';
  chatEl.classList.add('clearing');
  setTimeout(async () => {
    // 摘下 empty-state 模板（innerHTML 清空会连带删掉它）
    const es = document.getElementById('empty-state');
    chatEl.innerHTML = '';
    runningTools = {};
    streamBubble = null;
    if (es) {
      es.classList.remove('hidden'); // 无历史 → 重新展示快捷指令
      chatEl.appendChild(es);
    }
    chatEl.classList.remove('clearing');
    delete chatEl.dataset.clearing;
    setPhase('idle');
    await invoke('reset_history'); // 后端归档会话精华入记忆
  }, 240);
});

document.getElementById('btn-settings').addEventListener('click', async () => {
  const cfg = await invoke('get_config');
  document.getElementById('cfg-key').value = cfg.api_key || '';
  document.getElementById('cfg-url').value = cfg.base_url || '';
  document.getElementById('cfg-model').value = cfg.model || '';
  document.getElementById('cfg-hotkey').value = cfg.hotkey || '';
  document.getElementById('cfg-tts').checked = !!cfg.tts_enabled;
  document.getElementById('cfg-autostart').checked = !!cfg.autostart;
  document.getElementById('cfg-topmost').checked = !!cfg.always_on_top;
  document.getElementById('cfg-vm').value = cfg.voice_model || 'fast';
  document.getElementById('cfg-vlang').value = cfg.voice_lang || 'zh';
  document.getElementById('cfg-cmds').value = (cfg.custom_cmds || []).join('\n');
  /* 识别服务状态（排障：路径缺失 / server 是否已预热） */
  const vs = document.getElementById('voice-status');
  if (vs) {
    vs.textContent = '识别服务：检测中…';
    invoke('probe_env').then((info) => {
      const m = /端口 (\d+)/.exec(String(info));
      vs.textContent = m && m[1] !== '0'
        ? '识别服务：运行中（端口 ' + m[1] + '）'
        : '识别服务：待命（首次语音时自动拉起）';
    }).catch(() => {
      vs.textContent = '识别服务：环境缺失（未找到 tools/whisper）';
    });
  }
  /* 长期记忆内容（可手动编辑，保存时一并提交） */
  try {
    const mem = await invoke('get_memory');
    document.getElementById('cfg-memory').value = mem;
  } catch (_) {}
  /* 显示运行中的真实版本号（排查“装的包 vs 跑的进程”不一致） */
  try {
    const ver = await TAURI.app.getVersion();
    document.getElementById('ver-text').textContent = 'v' + ver;
  } catch (_) { /* 版本获取失败不阻塞设置面板 */ }
  document.getElementById('settings').classList.remove('hidden');
});

document.getElementById('btn-close-settings').addEventListener('click', () => {
  document.getElementById('settings').classList.add('hidden');
});

document.getElementById('btn-save-settings').addEventListener('click', async () => {
  const status = document.getElementById('settings-status');
  status.textContent = '';
  const autostart = document.getElementById('cfg-autostart').checked;
  const topmost = document.getElementById('cfg-topmost').checked;
  const tts = document.getElementById('cfg-tts').checked;
  ttsOn = tts; // 即时生效；关闭时停掉在读的
  if (!tts) stopSpeak();
  try {
    await invoke('save_config', {
      config: {
        api_key: document.getElementById('cfg-key').value.trim(),
        base_url: document.getElementById('cfg-url').value.trim(),
        model: document.getElementById('cfg-model').value.trim(),
        hotkey: document.getElementById('cfg-hotkey').value.trim(),
        autostart,
        always_on_top: topmost,
        voice_model: document.getElementById('cfg-vm').value,
        voice_threads: cfg.voice_threads || 0,
        voice_lang: document.getElementById('cfg-vlang').value,
        custom_cmds: document.getElementById('cfg-cmds').value.split('\n')
          .map((s) => s.trim()).filter(Boolean).slice(0, 8),
        tts_enabled: tts,
      },
    });
    // 长期记忆（面板文本可手动编辑）
    await invoke('save_memory_cmd', { summary: document.getElementById('cfg-memory').value.trim() });
    // 保存即生效（不等重启）
    await invoke('set_autostart', { on: autostart });
    invoke('set_always_on_top', { on: topmost });
    status.textContent = '✓ 已保存';
    // 快捷指令即时生效（空态有胶囊时重建）
    currentCmds = document.getElementById('cfg-cmds').value.split('\n')
      .map((s) => s.trim()).filter(Boolean).slice(0, 8);
    const esNow = document.getElementById('empty-state');
    if (esNow) renderChips(esNow, currentCmds.length ? currentCmds : DEFAULT_CMDS);
    // 配完 Key 回到界面：若无任何消息则把空态请回来（新手闭环）
    if (!document.querySelector('#chat .bubble')) {
      const es2 = document.getElementById('empty-state');
      if (es2) {
        renderChips(es2, currentCmds.length ? currentCmds : DEFAULT_CMDS);
        es2.classList.remove('hidden');
      }
    }
    setTimeout(() => {
      document.getElementById('settings').classList.add('hidden');
      status.textContent = '';
    }, 900);
  } catch (e) {
    status.textContent = '保存失败：' + e;
  }
});

document.getElementById('btn-clear-memory').addEventListener('click', async () => {
  document.getElementById('cfg-memory').value = '';
  try { await invoke('save_memory_cmd', { summary: '' }); } catch (_) {}
  const status = document.getElementById('settings-status');
  status.textContent = '✓ 记忆已清除';
  setTimeout(() => { status.textContent = ''; }, 1500);
});

/* ---------- 启动 ---------- */
window.addEventListener('DOMContentLoaded', async () => {
  const cfg = await invoke('get_config');
  const demo = new URLSearchParams(location.search).get('demo');
  if (!cfg.api_key && !demo) {
    addBubble('ai', '你好，我是 VCC ⚡\n首次使用请先点右上角 ⚙ 填入 DeepSeek API Key。\n\n可以用语音或文字让我：调音量/亮度、点鼠标、开文件、跑命令…');
  }
  // 恢复上次对话（只渲染 user/assistant 文本气泡，工具中间轮不进 UI）
  try {
    const hist = await invoke('load_chat_history');
    const visible = hist.filter((m) =>
      (m.role === 'user' || m.role === 'assistant') &&
      typeof m.content === 'string' && m.content.trim());
    if (visible.length) {
      addDivider('上次对话');
      for (const m of visible) addBubble(m.role === 'user' ? 'user' : 'ai', m.content);
    }
  } catch (_) { /* 历史加载失败不阻塞启动 */ }
  // 空状态：无历史无消息时展示快捷指令（演示模式除外）
  const es = document.getElementById('empty-state');
  if (es && !demo && !document.querySelector('#chat .bubble')) {
    currentCmds = (cfg.custom_cmds && cfg.custom_cmds.length) ? cfg.custom_cmds : DEFAULT_CMDS;
    renderChips(es, currentCmds);
    es.classList.remove('hidden');
  }
  ttsOn = !!cfg.tts_enabled;
  setPhase('idle');

  /* 拖文件进窗口 = 打开它（课堂：课件直接拖进来） */
  try {
    const wv = TAURI.webview.getCurrentWebview();
    await wv.onDragDropEvent((event) => {
      if (event.payload.type === 'drop' && event.payload.paths && event.payload.paths.length) {
        send('打开 ' + event.payload.paths[0]);
      }
    });
  } catch (_) { /* 拖拽不可用静默 */ }

  /* 演示模式：index.html?demo=listening|thinking|executing 供截图对比 */
  if (demo) {
    document.body.classList.add('preview');
    addBubble('user', '把音量调到 30，然后打开 D 盘的课件文件夹');
    const t1 = addToolLine('设置系统音量 → 30%');
    finishToolLine(t1, true, 812);
    const t2 = addToolLine('打开路径 D:\\课件');
    finishToolLine(t2, true, 234);
    const t3 = addToolLine('读取文件夹列表');
    if (demo === 'executing') {
      showTranscript('把音量调到三十，打开课件文件夹');
      setPhase('executing');
    } else if (demo === 'thinking') {
      setPhase('thinking');
    } else if (demo === 'done') {
      finishToolLine(t3, true);
      addBubble('ai', '好的，音量已调到 30%。「课件」文件夹已在资源管理器中打开。');
      setPhase('done');
    } else if (demo === 'listening') {
      showTranscript('把音量调到三十');
      setPhase('listening');
    } else {
      finishToolLine(t3, true);
      addBubble('ai', '好的，音量已调到 30%。「课件」文件夹已在资源管理器中打开，里面共有 6 个文件，需要我演示其中某个吗？');
      setPhase('idle');
    }
  }
});
