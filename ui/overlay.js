/* ============ VCC 底部波形跑马灯 — 小布 Next「长按波形」COE 完整还原 ============ */
/* 来源：com.oplus.claw 17.0.72 assets/cui_bottom-918.coz（COE 特效引擎）
   - 片元着色器：coe_out/layer0_render0.fs.glsl 316 行 GLSL ES 300 逐行移植（WebGL2）
   - uniform 默认值：coe_scene.json wave_final.render[0].uniforms 全表照搬
   - 进出动画轨：COE FadeIn 624ms / FadeOut 464ms，bezier(0.33,0,0.67,1)；
     声浪基线 (0.15,0.30,0.15) 688ms；边框呼吸 (1,0.3,0.6,0.5)
   - LUT：textures/color_tex_3240x1.png 色环（REPEAT 循环滚动=跑马灯核心）、
          textures/noise_tex.png 边框流光（MIRRORED_REPEAT）
   - 三层波：层1 曲线波（freq7/speed3.3 硬边）、层2 辉光波（freq15/speed4.3）、
     层3 噪声波（freq15/speed2/羽化 0.09）；颜色各取色环 1/3 段错开滚动 */

const canvas = document.getElementById('gl');
const CTX_OPTS = { alpha: true, premultipliedAlpha: true, antialias: false, depth: false, stencil: false };
const FORCE_GL1 = /[?&]gl1/.test(location.search);   // 无头验证回退路径用
let gl = FORCE_GL1 ? null : canvas.getContext('webgl2', CTX_OPTS);
let gl1 = false;   // WebGL1 回退：真机 WebView2 分层透明窗口下 webgl2 可能创建失败（旧版 fbm 即 webgl1）
if (!gl) {
  gl = canvas.getContext('webgl', CTX_OPTS) || canvas.getContext('experimental-webgl', CTX_OPTS);
  gl1 = !!gl;
}
if (!gl) {
  document.body.classList.add('no-webgl');   // CSS 兜底呼吸条：保证课堂场景可见反馈
  if (typeof dbg === 'function') dbg('WebGL unavailable, CSS fallback');
  console.error('[vcc-wave] WebGL unavailable, CSS fallback');
} else {
  if (typeof dbg === 'function' && /[?&]debug/.test(location.search)) dbg('context: ' + (gl1 ? 'webgl1 (fallback)' : 'webgl2'));

const VERT = `#version 300 es
in vec2 a_position;
out vec2 v_texCoord;
void main() {
    gl_Position = vec4(a_position, 0.0, 1.0);
    v_texCoord = a_position * 0.5 + 0.5;
}
`;

/* ======== 原文：layer0_render0.fs.glsl（除本注释外逐行未动） ======== */
const FRAG = `#version 300 es
precision highp float;

in vec2 v_texCoord;
out vec4 fragColor;

// @Vec2(iResolution, 0.1)=[1920, 1080]
uniform vec2 u_resolution;
// @Range(iTime, 0, 1000, 0) = 0.0
uniform float u_time;
// @Texture(colorTex)=[1,1,0,0]
uniform sampler2D colorTex;
// @Vec2(水波缩放, 0.01)=[1.0,1.0]
uniform vec2 u_scale;
// @Vec2(水波位置,0.01)=[0.0,0.0]
uniform vec2 u_pos;
// @Range(水波透明度, 0.0, 1.0, 0.01)=1.0
uniform float u_wave_alpha;
// @Range(水波整体高度, 0, 5, 0.1) = 1.0
uniform float totalLevel;
// @Range(水波跟随高度, 0, 5, 0.1) = 1.0
uniform float rectFactor;
// @Range(colorTexSwitch, 0, 1, 1) = 0.
uniform float colorTexSwitch;
// @Range(soundChange, 0, 2, 0.01) = 0.0
uniform float soundChange;

// @Label(wave1)
// @Range(brightness1, 0, 2, 0.01) = 1.0
uniform float brightness1;
// @Range(soundLevel, 0, 2, 0.01) = 0.5
uniform float u_soundLevel1;
// @Range(频率, 1, 50, 1) = 15.0
uniform float u_frequency1;
// @Range(速度, 0, 10, 0.1) = 2.0
uniform float u_speed1;
// @Range(锐利度, 1, 10, 0.1) = 2.0
uniform float u_sharpness1;
// @Range(音浪衰减, 0.0, 1.0, 0.01) = 0.5
uniform float u_waveAttenuation1;

// @Label(wave2)
// @Range(brightness2, 0, 2, 0.01) = 1.0
uniform float brightness2;
// @Range(soundLevel2, 0, 2, 0.01) = 0.5
uniform float u_soundLevel2;
// @Range(频率2, 1, 50, 1) = 15.0
uniform float u_frequency2;
// @Range(速度2, 0, 10, 0.1) = 2.0
uniform float u_speed2;
// @Range(锐利度, 0.0, 1.0, 0.01) = 0.05
uniform float u_sharpness2;
// @Range(音浪衰减, 0.0, 1.0, 0.01) = 0.5
uniform float u_waveAttenuation2;

// @Label(wave3)
// @Range(brightness3, 0, 2, 0.01) = 1.0
uniform float brightness3;
// @Range(soundLevel3, 0, 2, 0.01) = 0.5
uniform float u_soundLevel3;
// @Range(频率3, 1, 50, 1) = 15.0
uniform float u_frequency3;
// @Range(速度3, 0, 10, 0.1) = 2.0
uniform float u_speed3;
// @Range(锐利度3, 0.0, 1.0, 0.01) = 0.05
uniform float u_sharpness3;
// @Range(音浪衰减, 0.0, 1.0, 0.01) = 0.5
uniform float u_waveAttenuation3;
// @Vec2(u_sound_rangX,1.0)=[10,10]
uniform vec2 u_sound_rangX;

// @Label(边框)
// @Range(u_density, 0.0, 10.0, 0.01)=3.0
uniform float u_density;
// @Vec2(u_rectSize,2.0)=[128,128]
uniform vec2 u_rectSize;
// @Range(u_rectCorner, 0.0, 200.0, 1.0)=4
uniform float u_rectCorner;
// @Vec3(u_border,1.0)=[2,0,2]
uniform vec3 u_border;
// @Vec3(u_borderColor,0.01)=[1.0,1.0,1.0]
uniform vec3 u_borderColor;
// @Vec3(u_border_light,1.0)=[2,10,10]
uniform vec3 u_border_light;
// @Vec4(u_borderAlpha, 0.01)=[1.0,1.0,1.0,1.0]
uniform vec4 u_borderAlpha;
// @Range(u_FadeAlpha,0.0,5.0,0.01)=0.5
uniform float u_FadeAlpha;
// @Texture(u_noiseTexture)=[1,1,0,0]
uniform sampler2D u_noiseTexture;
// @Range(u_flowSpeed,0.0,10.0,0.01)=0.0
uniform float u_flowSpeed;
// @Vec3(u_blackColor, 0.01)=[1.0,1.0,1.0]
uniform vec3 u_blackColor;
// @Vec2(u_blackAlpha,0.01)=[0.1,0.7]
uniform vec2 u_blackAlpha;
// @Vec2(u_blackPos,0.01)=[0.1,0.7]
uniform vec2 u_blackPos;

const float PI = 3.1415926;
float random(vec2 seed) {
    return fract(sin(dot(seed, vec2(12.9898, 78.233))) * 43758.5453);
}
float hash(float n) {
    return fract(sin(n) * 43758.5453123);
}
float grad(float x) {
    return hash(x) * 2.0 - 1.0;
}
float fade(float t) {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}
float gradientNoise(float p) {
    float i = floor(p);
    float f = fract(p);
    float g0 = grad(i);
    float g1 = grad(i + 1.0);
    float v0 = g0 * f;
    float v1 = g1 * (f - 1.0);
    return mix(v0, v1, fade(f));
}

float createWave(vec2 uv, float time, float frequency, float speed) {
    float wave = 0.0;
    wave += sin(uv.x * frequency * 1.0 + time * speed) * 0.4;
    wave += gradientNoise(uv.x);
    return wave;
}

float createWaveCurve(vec2 uv, float time, float frequency, float speed) {
    float wave = 0.0;
    wave += sin(uv.x * frequency * 1.0 + time * speed) * 0.4;
    wave += cos(uv.x * frequency * 1.5 + time * speed * 0.8) * 0.2;
    wave += sin(uv.x * frequency * 2.2 + time * speed * 1.2) * 0.1;
    return wave;
}

float isJitterActive(float duration, float time) {
    float cycleDuration = duration;
    float cycleIndex = floor(time / cycleDuration);
    vec2 seed = vec2(cycleIndex, 0.0);
    // float activeRatio = 0.1 + 0.7 * random(seed);
    float activeRatio = 1.0;
    float activeDuration = cycleDuration * activeRatio;
    float cycleTime = mod(time, cycleDuration);
    float fadeIn = smoothstep(0.0, activeDuration/2.0, cycleTime);
    float fadeOut = smoothstep(activeDuration, activeDuration/2.0, cycleTime);
    return step(cycleTime, activeDuration) * fadeIn * fadeOut;
}

vec4 showWave(int index, vec2 uv, float colorOffset, float soundLevel, float waveAttenuation,
            float frequency, float speed, float sharpness) {
    vec3 finalColor = vec3(0.0);
    float finalAlpha = 0.0;
    float progress = 2.0;
    vec2 centeredUV = vec2(uv.x, (uv.y * 2.0 - 1.0));
    float time = u_time * (1.0 + progress * 0.2);
    float level = soundLevel * (1.0 - progress * waveAttenuation) * totalLevel;

    float wave = 0.0;
    if (index <= 1) {
        wave = createWaveCurve(uv, time, frequency, speed);
    } else {
        wave = createWave(uv, time, frequency, speed);
    }
    // float sharpenTrail = 1.0 - smoothstep(.1, 0.5, abs(centeredUV.x - 0.5));
    float left = smoothstep(-0.2, 0.2, uv.x);
    float right = smoothstep(1.2, 0.8, uv.x);
    float finalWave = wave * level * left;
    float dist = abs(centeredUV.y) - (0.0 + abs(finalWave));
    vec2 colorUV = uv;
    float colorSpeed = u_speed3/4.0;
    float timeCycle = fract(colorUV.x + u_time * colorSpeed);
    colorUV.x = timeCycle * (1.0 / 3.0) + colorOffset / 3.0;
    float texY = 0.4;
    if (colorTexSwitch > 0.5) {
        texY = 0.6;
    }
    // vec4 col = texture(colorTex, (vec2(colorUV.x, texY)));
    vec4 col = texture(colorTex, colorUV);

    float alpha = 1.0;
    if (index == 1) {
        // alpha = step(dist, 0.0);
        alpha = smoothstep(0.05, 0.0, dist);
    } else if(index == 2) {
        alpha = smoothstep(0.01, -0.01, dist);
        float glowSize = 0.2 * u_sharpness2;
        float glow = smoothstep(glowSize, 0.0, abs(dist));
        glow *= pow(glow, 2.0) * u_soundLevel2 * 0.8;
        alpha = max(alpha, glow);
        finalColor = mix(finalColor, vec3(1.0), glow * 0.6);
    } else {
        alpha = smoothstep(0.0, -0.1, dist);
        alpha += smoothstep(sharpness, -sharpness, dist);
    }
    finalColor = col.rgb;
    finalAlpha = max(finalAlpha, alpha);
    return vec4(finalColor, finalAlpha);
}

float sdRect(vec2 p, vec2 b, float r) {
    vec2 d = abs(p) - b + vec2(r);
    return min(max(d.x, d.y), 0.0) + length(max(d, 0.0)) - r;
}

const vec2 TEX_SIZE=vec2(512.,512.);
mediump vec4 getColorFromCircle(vec2 pos,vec2 size){
    float scale_=1.>size.x/size.y?TEX_SIZE.y/size.y:TEX_SIZE.x/size.x;
    vec2 uv=pos*scale_/TEX_SIZE+vec2(.5);
    return texture(u_noiseTexture,uv);
}

vec2 rotate2d(vec2 uv,float angle){
    // float angle_r=radians(mod(angle,360.));
    float angle_r=mod(angle,6.28);
    // float angle_r=angle;
    mat2 mat=mat2(cos(angle_r),-sin(angle_r),sin(angle_r),cos(angle_r));
    return(mat*uv);
}

vec3 reinhardToneMapping(vec3 color) {
    return color / (color + vec3(1.0));
}

void main() {
    vec2 uv = gl_FragCoord.xy - 0.5*u_resolution;
    uv = uv/(vec2(u_rectSize.x, min(u_rectSize.y, 400.0)) * vec2(1.0, rectFactor)) + 0.5;

    uv /= u_scale;
    uv += u_pos;

    vec4 color1 = showWave(1, uv, 2.0, u_soundLevel1, u_waveAttenuation1, u_frequency1, u_speed1, u_sharpness1);
    vec4 color2 = showWave(2, uv, 0.0, u_soundLevel2, u_waveAttenuation2, u_frequency2, u_speed2, u_sharpness2);
    vec4 color3 = showWave(3, uv, 1.0, u_soundLevel3, u_waveAttenuation3, u_frequency3, u_speed3, u_sharpness3);

    color1.rgb *= color1.a;
    color2.rgb *= color2.a;
    color3.rgb *= color3.a;

    // 加法混合RGB（保留Alpha用于后续混合）
    vec3 blendedRGB = (color1.rgb * brightness1 + color2.rgb * brightness2 + color3.rgb * brightness3);
    float blendedAlpha = max(color1.a, max(color2.a, color3.a)); // 取最大Alpha
    vec4 soundColor = vec4(blendedRGB, blendedAlpha) * u_wave_alpha;

    //边框
    float globalDensity=u_density*.34;
    vec2 centerPos = gl_FragCoord.xy - u_resolution*0.5;

    //soundColor *= smoothstep(0.9, 0.7, abs((uv.x - 0.5) * 2.0));
    vec2 sound_rang_X = u_sound_rangX;
    sound_rang_X.y += sound_rang_X.x;
    sound_rang_X *= globalDensity;
    vec2 focusHalfSize = u_rectSize*0.5;

    soundColor *= smoothstep(focusHalfSize.x - sound_rang_X.x, focusHalfSize.x -sound_rang_X.y, abs((centerPos.x)));

    vec3 border = u_border;
    border.yz += u_border.x;
    border.z += u_border.y;
    border *= globalDensity;

    vec3 border_light = u_border_light;
    border_light.xyz += u_border.x + u_border.y;
    border_light *= globalDensity;

    float margin = max(border.z, max(max(border_light.x, border_light.y), border_light.z));
    vec2 max_=focusHalfSize;
    vec2 min_=focusHalfSize-vec2(max(.41*u_rectCorner+.71*margin,margin));
    vec4 color = vec4(0.0);
    float blackValue = 0.0;
    float grad_black = 0.0;

    if(all(greaterThan(max_,abs(centerPos)))){
        blackValue = 1.0;
        if(any(greaterThan(u_blackAlpha,vec2(0.001)))){
            float pos_y = (centerPos.y/focusHalfSize.y+1.0)*0.5;
            float blackMask = smoothstep(u_blackPos.x+u_blackPos.y, u_blackPos.x,pos_y);
            blackMask = blackMask*(u_blackAlpha.x-u_blackAlpha.y) + u_blackAlpha.y;
            grad_black = blackMask;
        }
        if(any(greaterThan(abs(centerPos),min_))){
            float fRect = sdRect(centerPos, focusHalfSize , u_rectCorner);
            blackValue = smoothstep(-border.y, -border_light.z, fRect);
            if(any(greaterThan(u_borderAlpha.xyz, vec3(0.001)))){
                float frameValue =  (smoothstep(0.0, -border.x, fRect) - smoothstep(-border.y, -border.z, fRect))*u_borderAlpha.x;
                float edge = step(-border.y,fRect);
                float light_w = smoothstep(0.0, -border.y, fRect)*edge*(u_borderAlpha.y+u_borderAlpha.z);
                float light_inner1 = smoothstep( -border_light.x, -border.y, fRect)*(1.0-edge)*u_borderAlpha.y;
                float light_inner2 = pow(smoothstep( -border_light.y, -border.y, fRect)*(1.0-edge), u_FadeAlpha)*u_borderAlpha.z;
                float light_inner = clamp(light_w+light_inner1+light_inner2, 0.0,1.0);
                if(any(greaterThan(vec2(frameValue,light_inner), vec2(0.001)))){
                    vec2 pos_color = rotate2d(centerPos*vec2(1.0,u_rectSize.x/u_rectSize.y),u_time*u_flowSpeed);
                    vec4 value_color = texture(u_noiseTexture,vec2(atan(pos_color.x, pos_color.y)/3.16*0.5+0.5, 0.5));
                    color = vec4(u_borderColor.rgb,1.0)*frameValue;
                    vec4 color_inner = vec4(1.0);
                    color_inner.rgb =  mix(u_borderColor.rgb, value_color.rgb, u_borderAlpha.w);
                    color_inner *= light_inner;
                    color.rgb  = color.rgb+color_inner.rgb*(1.0 - color.a);
                    color.a = color.a+color_inner.a*(1.0 - color.a);
                }
            }
        }
    }

    grad_black*=blackValue;
    color = clamp(color+soundColor, vec4(0.0), vec4(1.0));
    if(grad_black>0.001){
        grad_black *= (1.0 - color.a);
        color.rgb = u_blackColor*grad_black + color.rgb;
        color.a = grad_black+color.a;
    }

    fragColor = color;
}
`;
/* ======== 原文结束 ======== */

/* WebGL1（ES 100）降级版：仅当 webgl2 上下文创建失败时使用，由 ES 300 原文机械转换 */
function toES100Vert(s) {
  return s.replace('#version 300 es', '')
          .replace('in vec2 a_position;', 'attribute vec2 a_position;')
          .replace('out vec2 v_texCoord;', 'varying vec2 v_texCoord;');
}
function toES100Frag(s) {
  return s.replace('#version 300 es', '')
          .replace('in vec2 v_texCoord;', 'varying vec2 v_texCoord;')
          .replace('out vec4 fragColor;', '')
          .replace('fragColor = color;', 'gl_FragColor = color;')
          .replace(/texture[(]/g, 'texture2D(');
}
const VERT_ES100 = toES100Vert(VERT);
const FRAG_ES100 = toES100Frag(FRAG);

/* 诊断：错误既进 console 也（在 ?debug=1 下）画到 DOM——无头截图与现场排障共用 */
function dbg(msg) {
  console.error('[vcc-wave]', msg);
  if (!/[?&]debug/.test(location.search)) return;
  let pre = document.getElementById('vcc-dbg');
  if (!pre) {
    pre = document.createElement('pre');
    pre.id = 'vcc-dbg';
    pre.style.cssText = 'position:fixed;top:0;left:0;z-index:9;color:#fff;background:rgba(130,0,0,.88);font:11px/1.45 Consolas,monospace;margin:0;padding:6px 10px;max-width:100vw;max-height:70vh;overflow:hidden;white-space:pre-wrap';
    document.body.appendChild(pre);
  }
  pre.textContent += msg + '\n';
}
function sh(type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    dbg('shader compile failed:\n' + gl.getShaderInfoLog(s));
  }
  return s;
}
const prog = gl.createProgram();
gl.attachShader(prog, sh(gl.VERTEX_SHADER, gl1 ? VERT_ES100 : VERT));
gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, gl1 ? FRAG_ES100 : FRAG));
gl.linkProgram(prog);
if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
  dbg('program link failed:\n' + gl.getProgramInfoLog(prog));
}
gl.useProgram(prog);
if (/[?&]debug/.test(location.search)) {
  const ext = gl.getExtension('WEBGL_debug_renderer_info');
  dbg('WebGL2 renderer: ' + (ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER)));
}
if (/[?&]probe/.test(location.search)) window.__probeOn = true;

/* 全屏三角（a_position 语义同原 vs：直通裁剪空间） */
const buf = gl.createBuffer();
gl.bindBuffer(gl.ARRAY_BUFFER, buf);
gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
const loc = gl.getAttribLocation(prog, 'a_position');
gl.enableVertexAttribArray(loc);
gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

const U = {};
for (const n of ['u_resolution','u_time','u_scale','u_pos','u_wave_alpha','totalLevel',
  'rectFactor','colorTexSwitch','soundChange',
  'brightness1','u_soundLevel1','u_frequency1','u_speed1','u_sharpness1','u_waveAttenuation1',
  'brightness2','u_soundLevel2','u_frequency2','u_speed2','u_sharpness2','u_waveAttenuation2',
  'brightness3','u_soundLevel3','u_frequency3','u_speed3','u_sharpness3','u_waveAttenuation3',
  'u_sound_rangX','u_density','u_rectSize','u_rectCorner','u_border','u_borderColor',
  'u_border_light','u_borderAlpha','u_FadeAlpha','u_flowSpeed','u_blackColor','u_blackAlpha','u_blackPos']) {
  U[n] = gl.getUniformLocation(prog, n);
}

/* ======== 场景默认值（coe_scene.json wave_final.render[0].uniforms 全表） ======== */
gl.uniform2f(U.u_scale, 1, 1);
gl.uniform2f(U.u_pos, 0, 0);
gl.uniform1f(U.totalLevel, 1);
gl.uniform1f(U.rectFactor, 1.5);
gl.uniform1f(U.colorTexSwitch, 0);
gl.uniform1f(U.soundChange, 0);
gl.uniform1f(U.brightness1, 1); gl.uniform1f(U.brightness2, 1); gl.uniform1f(U.brightness3, 1);
gl.uniform1f(U.u_frequency1, 7);  gl.uniform1f(U.u_speed1, 3.3); gl.uniform1f(U.u_sharpness1, 2);
gl.uniform1f(U.u_frequency2, 15); gl.uniform1f(U.u_speed2, 4.3); gl.uniform1f(U.u_sharpness2, 2);
gl.uniform1f(U.u_frequency3, 15); gl.uniform1f(U.u_speed3, 2);   gl.uniform1f(U.u_sharpness3, 0.09);
gl.uniform1f(U.u_waveAttenuation1, 1); gl.uniform1f(U.u_waveAttenuation2, 1); gl.uniform1f(U.u_waveAttenuation3, 1);
gl.uniform2f(U.u_sound_rangX, 10, 200);
gl.uniform1f(U.u_density, 3);
gl.uniform3f(U.u_border, 2, 0, 2);
gl.uniform3f(U.u_borderColor, 1, 1, 1);
gl.uniform3f(U.u_border_light, 45, 20, 20);
gl.uniform1f(U.u_FadeAlpha, 2.18);
gl.uniform1f(U.u_flowSpeed, 1.5);
gl.uniform3f(U.u_blackColor, 0, 0, 0);
gl.uniform2f(U.u_blackAlpha, 0, 0);   // 黑幕关闭（透明 overlay 不压黑）
gl.uniform2f(U.u_blackPos, 0.1, 0.8);

/* ======== LUT 纹理（COE 包原版 PNG） ======== */
/* colorTex wrapMode=10497(GL_REPEAT) 色环无缝滚动；noise wrapMode=33648(GL_MIRRORED_REPEAT) */
function loadLut(b64, wrap, unit) {
  if (gl1) wrap = gl.CLAMP_TO_EDGE;   // WebGL1 NPOT 限制：色环 UV 由 fract 预包裹，CLAMP 无视觉损失
  const t = gl.createTexture();
  gl.activeTexture(unit);
  gl.bindTexture(gl.TEXTURE_2D, t);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, wrap);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, wrap);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([0, 0, 0, 0]));
  const img = new Image();
  img.onload = () => {
    window.__lut = (window.__lut || 0) + 1;   // 探针：LUT 已上传数
    gl.activeTexture(unit);   // 异步上传回各自单元，避免覆盖对方绑定
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
    gl.activeTexture(gl.TEXTURE0);
  };
  img.src = 'data:image/png;base64,' + b64;
  return t;
}
const LUT_COLOR_B64 = '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';
const LUT_NOISE_B64 = 'iVBORw0KGgoAAAANSUhEUgAAAhwAAAABCAYAAACVKXcQAAAACXBIWXMAAAsTAAALEwEAmpwYAAAAAXNSR0IArs4c6QAAAARnQU1BAACxjwv8YQUAAALMSURBVHgBZVVZtuM6CAQpd01vRW//39exRdeAHKf75CggKIpBtpz1/3+r4hUrXhk1i7JiBmzSoyUxVTM+e2J+orGMD8dP+H4K9qSN/gBuIbbSHOJiTNuifjoO+5KvYM+lfOQi/0j58hHbPCHelzkgYUPujGtErMxakFdWUqcNC3rBhtTo+hzwjSQGeMdIn5GX9sE46deAPf7xi+tCSQuMF0oFv6Rw4Pa+/ZP2jhtbkrfMt2sZrnepdmPt3zkrTnI6hrHuNdiP8Hcdku4Xf1FIhy6lR2pl0CWJCdLO3VjGCRuOUwyRq+zjb335IROyYjCh4qu5Y+dRKdEcH1wvolYpzyiekvE8WLUhCeTmA8/mGDt2686Nn3kwmpiIn/BP68jfenlpjwNpWw7bi3r7i08e5rNj1PsXx/rm3JjXxpovHnrOh++l3laOOOJeSfkrmXXgiT/Q1JHbNupdI6EXsb+ZKQx4HDfjDfxvmkc29Hgg0YGc4Epzwdb64UMcfTCDr3Tv8Zzq4DlQrT5wHvaMbLyHcOPqIesTxwPftvnY8zUf/WDe+OhaNj45bMdMyvzUSy7UedbMCxusOBFwoWvqF7o+MbGFO+S8baPkxx1kiT3ulG//CNmibVrGUZ66gyjtY/6lXJZdxx0rHXV0Ta4FT8rVdt6lZ4xPLmEGbMaAU/2dN362j5yW9NH+Bm5xHims80sXj9/V4BgXb3Q8A0t7sOLZ9B4jhY5JsEsdkTBgw+2p21k3r271JL4Ys4QjvrHcZ0vnaH2G40CiGsipeOklfn4RcvPuulxnKHdzjP56sF7yCcPaG+vHb+mLlfY1j2vUO6heli6Q7sX6rpf7qvz0d91zuaUwPKnGcp76mlTb6FNPX77u78G3czxr4gnLv2tmX3+dD2eymr/ajlPne+L5cTbZsRXv/hAdfIHwjnnZjg+P1jssb186hpj3thPn+D+QJLu6ZFkm2wAAAABJRU5ErkJggg==';
const texColor = loadLut(LUT_COLOR_B64, gl.REPEAT, gl.TEXTURE0);
const texNoise = loadLut(LUT_NOISE_B64, gl.MIRRORED_REPEAT, gl.TEXTURE1);
gl.uniform1i(gl.getUniformLocation(prog, 'colorTex'), 0);
gl.uniform1i(gl.getUniformLocation(prog, 'u_noiseTexture'), 1);

gl.enable(gl.BLEND);
gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);   // COE blendSfactor=1 / blendDfactor=771 预乘混合

/* ======== 布局（COE layout=[0,1200,1080,300]、u_rectSize=[984,268]、u_rectCorner=72 等比） ======== */
/* 条带高 160 逻辑 px ≈ 手机 52dp 视图 @2.75x；rect/条带比例与 COE 完全一致 */
const STRIP_H = 160;
function resize() {
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  const w = Math.floor(innerWidth * dpr);
  const h = Math.floor(STRIP_H * dpr);
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  gl.viewport(0, 0, w, h);
  const rectW = w * 984 / 1080;
  const rectH = h * 268 / 300;
  gl.uniform2f(U.u_resolution, w, h);
  gl.uniform2f(U.u_rectSize, rectW, rectH);
  gl.uniform1f(U.u_rectCorner, 72 * rectH / 268);
}
addEventListener('resize', resize);
resize();

/* ======== COE 动画轨：bezier(0.33,0,0.67,1)，进出不对称（进入 624/688ms，退出 464ms） ======== */
function bezEval(x1, y1, x2, y2, t) {
  if (t <= 0) return 0;
  if (t >= 1) return 1;
  const cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx;
  const cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by;
  let x = t;
  for (let i = 0; i < 4; i++) {
    const f = ((ax * x + bx) * x + cx) * x - t;
    if (Math.abs(f) < 1e-5) break;
    const d = (3 * ax * x + 2 * bx) * x + cx;
    if (Math.abs(d) < 1e-6) break;
    x -= f / d;
  }
  return ((ay * x + by) * x + cy) * x;
}
const EASE = [0.33, 0, 0.67, 1];
const FADE_IN_A = 0.624, FADE_IN_L = 0.688, FADE_OUT_A = 0.464, FADE_OUT_L = 0.464;
const LVL_BASE = [0.15, 0.301, 0.15];          // COE FadeOut/FadeIn 声浪基线
const BORDER_ON = [1, 0.3, 0.6, 0.5];          // COE 边框呼吸终值（w=0.5 混流光）

/* 每通道独立补间，支持中断续值（从当前显示值出发，COUI 中断规则） */
function chan() { return { v: 0, from: 0, to: 0, t: 1, dur: 1, t0: 0 }; }
const chAlpha = chan();
const chBase = [chan(), chan(), chan()];
const chBorder = [chan(), chan(), chan(), chan()];
function seek(ch, target, dur) {
  ch.from = ch.v; ch.to = target; ch.t = 0; ch.dur = Math.max(dur, 1e-4); ch.t0 = performance.now();
}
/* 绝对时间推进：rAF 被节流（窗口隐藏/遮挡）时也能走到正确进度，显隐绝不卡在透明态 */
function tick(ch, nowMs) {
  if (ch.t >= 1) return;
  ch.t = Math.min(1, (nowMs - ch.t0) / (ch.dur * 1000));
  ch.v = ch.from + (ch.to - ch.from) * bezEval(EASE[0], EASE[1], EASE[2], EASE[3], ch.t);
}

/* ======== phase → 条带显隐 + 声浪驱动 ======== */
/* 显示集（Rust 侧同步）：listening / executing / done；其余相位淡出，950ms 后窗口隐藏 */
const BAND_ON = new Set(['listening', 'executing', 'done']);
let phase = 'idle';
let lvlTarget = 0, lvl = 0;          // 麦克风电平包络（快攻慢放）
let tAcc = 0, last = performance.now();

function setPhase(p) {
  if (!BAND_ON.has(p) && !BAND_ON.has(phase) && p === phase) return;
  const wasOn = BAND_ON.has(phase);
  const isOn = BAND_ON.has(p);
  phase = p;
  if (isOn && !wasOn) {
    seek(chAlpha, 1, FADE_IN_A);
    for (let i = 0; i < 3; i++) seek(chBase[i], LVL_BASE[i], FADE_IN_L);
    for (let i = 0; i < 4; i++) seek(chBorder[i], BORDER_ON[i], FADE_IN_A);
  } else if (!isOn && (wasOn || chAlpha.to !== 0)) {
    seek(chAlpha, 0, FADE_OUT_A);
    for (let i = 0; i < 3; i++) seek(chBase[i], 0, FADE_OUT_L);
    for (let i = 0; i < 4; i++) seek(chBorder[i], 0, FADE_OUT_L);
  }
  canvas.classList.toggle('on', isOn);
}

/* 执行/收尾期无麦克风流 → 程序化伪语音包络（层叠正弦 + 抖动，模拟说话起伏） */
function proceduralLevel(t) {
  const v = 0.42 + 0.16 * Math.sin(2.31 * t) + 0.11 * Math.sin(3.73 * t + 1.7)
          + 0.07 * Math.sin(7.1 * t + 0.6) + 0.05 * Math.sin(11.4 * t + 2.1);
  return Math.min(0.9, Math.max(0.05, v));
}

let frameNo = 0;
function probeDump() {
  const cx = Math.floor(canvas.width / 2);
  const px = new Uint8Array(4);
  const hits = [];
  let maxA = 0;
  for (let y = 0; y < canvas.height; y++) {
    gl.readPixels(cx, y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
    if (px[3] > 2) { hits.push(y + ':' + px.join(',')); if (px[3] > maxA) maxA = px[3]; }
  }
  dbg('PROBE phase=' + phase + ' tAcc=' + tAcc.toFixed(2) +
      ' alpha=' + chAlpha.v.toFixed(3) + ' border0=' + chBorder[0].v.toFixed(3) +
      ' lvl=' + lvl.toFixed(3) + ' lut=' + (window.__lut || 0) +
      ' maxA=' + maxA + ' hits[' + hits.length + ']=' + hits.slice(0, 10).join(' | '));
}
function drawFrame() {
  let live = 0;
  if (phase === 'listening') live = lvl;
  else if (phase === 'executing') live = proceduralLevel(tAcc);
  else if (phase === 'done') live = 0.85 + 0.08 * Math.sin(9.0 * tAcc);   // 收尾绽放：短暂推高
  gl.uniform1f(U.u_soundLevel1, Math.min(1.8, chBase[0].v + live * 1.00));
  gl.uniform1f(U.u_soundLevel2, Math.min(1.8, chBase[1].v + live * 0.75));
  gl.uniform1f(U.u_soundLevel3, Math.min(1.8, chBase[2].v + live * 0.55));
  gl.uniform1f(U.u_wave_alpha, chAlpha.v);
  gl.uniform4f(U.u_borderAlpha, chBorder[0].v, chBorder[1].v, chBorder[2].v, chBorder[3].v);
  gl.uniform1f(U.u_time, tAcc);

  resize();
  gl.clearColor(0, 0, 0, 0);
  gl.clear(gl.COLOR_BUFFER_BIT);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
}

function frame(now) {
  const dt = Math.min((now - last) / 1000, 0.1);
  last = now;
  tAcc += dt;
  tick(chAlpha, now);
  for (const c of chBase) tick(c, now);
  for (const c of chBorder) tick(c, now);

  /* 语音电平包络：快攻慢放（经典音频表头手法） */
  const k = lvlTarget > lvl ? 1 - Math.exp(-dt * 22) : 1 - Math.exp(-dt * 4.5);
  lvl += (lvlTarget - lvl) * k;

  drawFrame();
  frameNo++;
  if (window.__probeOn && frameNo === 20) probeDump();
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

/* ======== Tauri(桥) 事件 / 演示参数 ======== */
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
if (qp.has('fast')) {   // 跳过渐变直接到终值（无头截图/渲染验证）
  chAlpha.v = chAlpha.to;
  for (const c of chBase) c.v = c.to;
  for (const c of chBorder) c.v = c.to;
}
if (qp.has('still')) {  // 单帧取证：固定 tAcc 画一帧 + 立即读回像素
  tAcc = 5.0;
  drawFrame();
  if (window.__probeOn) probeDump();
}

}
