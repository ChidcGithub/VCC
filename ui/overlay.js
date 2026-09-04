/* ============ VCC 全屏跑马灯 — WebGL 实时波浪折射光环 v2 ============ */
/* 对标 Apple Intelligence (iOS 18-26 Siri glow)：
   - 七色粉彩板（社区公认采样）沿屏幕边缘环形分布
   - 双层结构：锐利核心光带 + 大范围柔光晕（错拍相位，独立律动）
   - 周期化 fbm 噪声实时置换（液体折射），白色扫光沿周长旅行
   - 语音响应：聆听态随麦克风电平呼吸（uLevel） */

const canvas = document.getElementById('gl');
const gl = canvas.getContext('webgl', {
  alpha: true, premultipliedAlpha: true,
  antialias: false, depth: false, stencil: false,
});

const VERT = `
attribute vec2 p;
void main() { gl_Position = vec4(p, 0.0, 1.0); }
`;

const FRAG = `
precision highp float;
uniform vec2 uRes;
uniform float uTime;
uniform float uInt;
uniform float uLevel;
uniform float uSpin;

float hash(vec2 p) {
  p = fract(p * vec2(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}
float vnoise(vec2 p) {
  vec2 i = floor(p), f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float a = hash(i);
  float b = hash(i + vec2(1.0, 0.0));
  float c = hash(i + vec2(0.0, 1.0));
  float d = hash(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
float fbm(vec2 p) {
  float v = 0.0;
  float a = 0.5;
  for (int i = 0; i < 4; i++) {
    v += a * vnoise(p);
    p = p * 2.02 + vec2(31.7, 17.3);
    a *= 0.5;
  }
  return v;
}
/* 圆角矩形 SDF：屏幕边缘为基准，光带贴边 */
float sdBox(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
/* Apple Intelligence 七色板（顺序排过：RGB 插值全程避开浑浊灰区）
   8D9FFF 蓝 → C686FF 浅紫 → BC82F3 紫 → F5B9EA 粉 → FFBA71 琥珀 → FF6778 珊瑚 → AA6EEE 紫罗兰 */
vec3 palette(float t) {
  vec3 c0 = vec3(0.553, 0.624, 1.000);
  vec3 c1 = vec3(0.776, 0.525, 1.000);
  vec3 c2 = vec3(0.737, 0.510, 0.953);
  vec3 c3 = vec3(0.961, 0.725, 0.918);
  vec3 c4 = vec3(1.000, 0.729, 0.443);
  vec3 c5 = vec3(1.000, 0.404, 0.471);
  vec3 c6 = vec3(0.667, 0.431, 0.933);
  t = fract(t) * 7.0;
  vec3 c = mix(c0, c1, clamp(t, 0.0, 1.0));
  c = mix(c, c2, clamp(t - 1.0, 0.0, 1.0));
  c = mix(c, c3, clamp(t - 2.0, 0.0, 1.0));
  c = mix(c, c4, clamp(t - 3.0, 0.0, 1.0));
  c = mix(c, c5, clamp(t - 4.0, 0.0, 1.0));
  c = mix(c, c6, clamp(t - 5.0, 0.0, 1.0));
  c = mix(c, c0, clamp(t - 6.0, 0.0, 1.0));
  return c;
}
/* 高斯光带：中心线被波浪位移（对称，备用） */
float gaussAt(float d, float center, float sigma) {
  float x = (d - center) / sigma;
  return exp(-x * x);
}
/* 不对称光带：外半边（d > center，朝屏幕物理边缘）厚而浓、铺满到边，
   内半边（d < center，朝屏幕中心）延展而渐淡 */
float gaussAsym(float d, float center, float sigIn, float sigOut) {
  float x = (d - center) / (d > center ? sigOut : sigIn);
  return exp(-x * x);
}

void main() {
  vec2 c2 = uRes * 0.5;
  vec2 q = gl_FragCoord.xy - c2;
  float t = uTime;

  float inset = 12.0;
  vec2 b = c2 - vec2(inset);
  float d = sdBox(q, b, 14.0);            /* < 0 在屏幕内侧；小圆角让光带拐角贴近物理角尖 */

  /* 语音响应：电平推高波浪振幅与亮度 */
  float amp = 1.0 + uLevel * 0.38;

  /* 周期坐标：单位圆方向（绕环无缝，不用 atan） */
  vec2 dir = q / max(length(q), 1e-4);

  /* 定向环流：长涌噪声场绕屏幕旋转（Siri thinking 的能量流动感）
     刚体旋转保持单位圆周期性 → 天然无缝 */
  float ra = t * uSpin;
  mat2 R = mat2(cos(ra), -sin(ra), sin(ra), cos(ra));
  vec2 rdir = R * dir;

  /* 核心层波浪：长涌（随环流旋转）+ 中浪 + 细碎折射纹（局部） */
  float w1 = fbm(rdir * 2.2 + vec2(t * 0.10, 3.0)) - 0.5;
  float w2 = fbm(dir * 4.5 + vec2(-t * 0.45, 9.0)) - 0.5;
  float w3 = fbm(dir * 9.0 + vec2(t * 0.8, 21.0)) - 0.5;
  float wave = (w1 * 1.15 + w2 * 0.42 + w3 * 0.14) * amp;

  /* 光晕层波浪：低频为主 + 独立相位（错拍律动，两层不齐步） */
  float s1 = fbm(rdir * 1.9 + vec2(t * 0.07 + 0.5, 7.0)) - 0.5;
  float s2 = fbm(dir * 3.6 + vec2(-t * 0.33, 15.0)) - 0.5;
  float waveB = (s1 * 1.0 + s2 * 0.30) * amp;

  float width = 44.0;

  /* 色散折射：RGB 三通道轻微不同相位 → 边缘真实折射彩边 */
  float ca = 0.05 * sin(t * 0.7 + dir.x * 12.0 + dir.y * 7.0);
  /* 核心光带：外半边厚而浓（铺满到屏幕物理边缘），内半边延展渐淡 */
  float coreC = -width * (0.50 + 0.80 * wave);
  float br = gaussAsym(d, coreC + ca * width, width * 0.42, width * 1.45);
  float bg = gaussAsym(d, coreC, width * 0.42, width * 1.45);
  float bb = gaussAsym(d, coreC - ca * width, width * 0.42, width * 1.45);
  float core = (br + bg + bb) / 3.0;

  /* 柔光晕：重心贴边，外半边大范围铺开，内半边深延展渐淡 */
  float bloomC = -width * (0.85 + 0.60 * waveB);
  float bloom = gaussAsym(d, bloomC, width * 1.55, width * 2.60);

  /* 亮脊线：光在液体边缘波峰上集中（贴核心内缘的镜面高光） */
  float cx = (d - coreC + width * 0.60) / (width * 0.15);
  float crest = exp(-cx * cx);

  /* 周长亮度呼吸斑块 */
  float bright = 0.75 + 0.5 * fbm(dir * 1.8 + vec2(-t * 0.13, 4.2));

  /* 周长标量（仅用于扫光与色板，fract 距离天然无缝） */
  float ang = atan(q.y, q.x) / 6.28318530718 + 0.5;

  /* 白色扫光波峰 ×2（沿周长旅行的镜面高光，一主一副） */
  float sp1 = fract(t * 0.085);
  float dd1 = abs(fract(ang - sp1)); dd1 = min(dd1, 1.0 - dd1);
  float sweep1 = exp(-dd1 * dd1 * 7000.0) * 0.60;
  float sp2 = fract(t * 0.085 + 0.5);
  float dd2 = abs(fract(ang - sp2)); dd2 = min(dd2, 1.0 - dd2);
  float sweep2 = exp(-dd2 * dd2 * 11000.0) * 0.32;
  float sweep = sweep1 + sweep2;

  /* 上缘略亮（macOS 光环气质，略收敛） */
  float bias = mix(0.85, 1.12, smoothstep(-c2.y, c2.y, q.y));

  /* 语音响应：亮度与不透明度随电平抬起（温和，不洗白色板） */
  float lvlB = 1.0 + uLevel * 0.55;

  vec3 pal = palette(ang + t * 0.03);
  vec3 coreCol = pal * bright * bias * lvlB;
  vec3 bloomCol = mix(pal, vec3(1.0), 0.22) * bright * bias;

  vec3 col = coreCol * core * (0.9 + 1.0 * sweep)
           + bloomCol * bloom * (0.46 + 0.12 * sweep) * lvlB
           + vec3(1.0) * sweep * 0.42 * bg
           + vec3(1.0) * crest * (0.15 + 0.5 * sweep) * bg * uInt;

  /* 光铺满到屏幕物理边界（含四角，直角盒 SDF 判定），1.5px 收口抗锯齿 */
  float dEdge = sdBox(q, c2, 0.0);
  float edge = 1.0 - smoothstep(-1.0, 1.0, dEdge);
  float alpha = clamp(
    (core * (0.62 + 0.55 * bright) * bias * (0.7 + 0.9 * sweep) * lvlB
     + bloom * 0.42 * (0.5 + 0.5 * bright)) * uInt,
    0.0, 1.0) * edge;

  /* 抖动去色带：柔光渐变在暗底上极易 banding，加 ±1 LSB 噪声 */
  float dith = (hash(gl_FragCoord.xy + fract(t) * 61.7) - 0.5) * (1.8 / 255.0);
  col += dith;
  alpha = clamp(alpha + dith, 0.0, 1.0);

  gl_FragColor = vec4(col * alpha, alpha);
}
`;

function sh(type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    console.error(gl.getShaderInfoLog(s));
  }
  return s;
}
const prog = gl.createProgram();
gl.attachShader(prog, sh(gl.VERTEX_SHADER, VERT));
gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FRAG));
gl.linkProgram(prog);
gl.useProgram(prog);

const buf = gl.createBuffer();
gl.bindBuffer(gl.ARRAY_BUFFER, buf);
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
const loc = gl.getAttribLocation(prog, 'p');
gl.enableVertexAttribArray(loc);
gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

const uRes = gl.getUniformLocation(prog, 'uRes');
const uTime = gl.getUniformLocation(prog, 'uTime');
const uInt = gl.getUniformLocation(prog, 'uInt');
const uLevel = gl.getUniformLocation(prog, 'uLevel');
const uSpin = gl.getUniformLocation(prog, 'uSpin');

gl.enable(gl.BLEND);
gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);

/* 渲染分辨率缩放：光环是低频内容，0.6x + 线性放大几乎无损，省 ~65% GPU */
const RENDER_SCALE = 0.6;

function resize() {
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const w = Math.floor(innerWidth * dpr * RENDER_SCALE);
  const h = Math.floor(innerHeight * dpr * RENDER_SCALE);
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  gl.viewport(0, 0, w, h);
}
addEventListener('resize', resize);
resize();

/* phase → [强度, 速度, 环流]；JS 端弹性插值，速度/环流变化不跳变
   done = 完成绽放（回答收尾时短暂爆亮再回落）
   环流 spin：波场绕屏幕定向旋转，thinking/executing 明显加速 */
const TARGETS = {
  idle:      [0.00, 0.50, 0.015],
  invoke:    [0.50, 0.55, 0.05],
  listening: [0.76, 0.85, 0.07],
  thinking:  [0.85, 1.05, 0.42],
  executing: [1.00, 2.30, 1.10],
  done:      [1.22, 1.40, 0.30],
};
let tgtI = 0, tgtS = 0.5, tgtR = 0.015, curI = 0, curS = 0.5, curR = 0.015;
let lvlTarget = 0, lvl = 0;
let tAcc = 0, last = performance.now();

function setPhase(p) {
  if (!TARGETS[p]) return;
  const [i, s, r] = TARGETS[p];
  tgtI = i;
  tgtS = s;
  tgtR = r;
  canvas.classList.toggle('on', p !== 'idle');
}

function frame(now) {
  const dt = Math.min((now - last) / 1000, 0.1);
  last = now;
  const k = 1 - Math.exp(-dt * 3.2);
  curI += (tgtI - curI) * k;
  curS += (tgtS - curS) * k;
  curR += (tgtR - curR) * k;
  /* 语音电平包络：快攻慢放（经典音频表头手法） */
  const lk = lvlTarget > lvl ? 1 - Math.exp(-dt * 22) : 1 - Math.exp(-dt * 4.5);
  lvl += (lvlTarget - lvl) * lk;
  tAcc += dt * Math.max(curS, 0.05);
  resize();
  gl.uniform2f(uRes, canvas.width, canvas.height);
  gl.uniform1f(uTime, tAcc);
  gl.uniform1f(uInt, curI);
  gl.uniform1f(uLevel, lvl);
  gl.uniform1f(uSpin, curR);
  gl.clearColor(0, 0, 0, 0);
  gl.clear(gl.COLOR_BUFFER_BIT);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

/* Tauri 事件 / 演示参数 */
const TAURI = window.__TAURI__;
if (TAURI && TAURI.event) {
  TAURI.event.listen('vcc://phase', (e) => setPhase(e.payload.phase));
  TAURI.event.listen('vcc://level', (e) => {
    const v = e.payload && e.payload.level;
    lvlTarget = Math.max(0, Math.min(1, Number(v) || 0));
  });
}
const qp = new URLSearchParams(location.search);
if (qp.get('bg') === 'dark') document.body.style.background = '#101014'; // 截图演示用
const lv0 = parseFloat(qp.get('level') || '0');
if (lv0 > 0) lvlTarget = Math.min(1, lv0); // 截图演示语音响应用
setPhase(qp.get('phase') || qp.get('demo') || 'idle');
