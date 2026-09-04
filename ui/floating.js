/* ============ VCC 悬浮窗逻辑 ============ */
const TAURI = window.__TAURI__;
const invoke = TAURI ? TAURI.core.invoke : (async () => {});
const listen = TAURI ? TAURI.event.listen : (async () => {});

const card = document.getElementById('float-card');
const stateEl = document.getElementById('f-state');
const stepsEl = document.getElementById('f-steps');
const textEl = document.getElementById('f-text');

let fadeTimer = null;

function resizeToFit() {
  try {
    if (!TAURI) return;
    const h = Math.ceil(card.getBoundingClientRect().bottom) + 12;
    const w = TAURI.window.getCurrentWindow();
    w.setSize(new TAURI.dpi.LogicalSize(340, Math.min(Math.max(h, 64), 420)));
  } catch (e) { /* 忽略 */ }
}

function render(payload) {
  clearTimeout(fadeTimer);
  card.classList.remove('fade');

  if (payload.mode === 'show' || payload.mode === 'done') {
    stepsEl.innerHTML = '';
    const steps = payload.steps || [];
    // SF Symbols 风格步骤图标（SVG）
    const ICONS = {
      running: '<svg class="s-spin" viewBox="0 0 16 16" fill="none"><circle cx="8" cy="8" r="6" stroke="currentColor" stroke-width="2.2" stroke-dasharray="27" stroke-dashoffset="9" stroke-linecap="round"/></svg>',
      done: '<svg viewBox="0 0 16 16" fill="none"><circle cx="8" cy="8" r="7" fill="currentColor" opacity="0.14"/><path d="M4.8 8.2l2.2 2.2 4.2-4.6" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>',
      fail: '<svg viewBox="0 0 16 16" fill="none"><circle cx="8" cy="8" r="7" fill="currentColor" opacity="0.14"/><path d="M5.4 5.4l5.2 5.2M10.6 5.4l-5.2 5.2" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>',
    };
    for (const s of steps) {
      const div = document.createElement('div');
      const st = s.status || 'done';
      div.className = 'f-step ' + st;
      div.innerHTML = '<span class="s-ico">' + (ICONS[st] || ICONS.done) + '</span><span class="s-label"></span>';
      div.querySelector('.s-label').textContent = s.label;
      stepsEl.appendChild(div);
    }
    if (payload.text) {
      textEl.textContent = payload.text;
      textEl.classList.remove('hidden');
    } else {
      textEl.classList.add('hidden');
    }
    stateEl.textContent = payload.mode === 'done' ? '完成' : '执行中';
    card.querySelector('.f-dot').classList.toggle('busy', payload.mode !== 'done');

    card.classList.remove('show');
    void card.offsetWidth;
    card.classList.add('show');
    resizeToFit();

    if (payload.mode === 'done') {
      // 完成 → 5 秒后淡出 → 隐藏窗口
      fadeTimer = setTimeout(async () => {
        card.classList.add('fade');
        setTimeout(() => invoke('hide_floating'), 450);
      }, 5000);
    }
  }
}

listen('vcc://float', (e) => render(e.payload));

/* 演示参数（截图管线用）：floating.html?demo=done|busy&bg=dark */
const qp = new URLSearchParams(location.search);
if (qp.get('bg') === 'dark') document.body.style.background = '#101014';
if (qp.get('demo')) {
  const busy = qp.get('demo') === 'busy';
  render({
    mode: busy ? 'show' : 'done',
    steps: [
      { label: '设置系统音量 → 30%', status: 'done' },
      { label: '打开路径 D:\\课件', status: 'done' },
      { label: '读取文件夹列表', status: busy ? 'running' : 'done' },
    ],
    text: '好的，音量已调到 30%。「课件」文件夹已在资源管理器中打开。',
  });
}
