/* VCC UI 截图管线：主窗口 / 全屏 overlay 状态批量截图 */
const { chromium } = require('playwright-core');
const path = require('path');

const OUT = 'D:/My things/Learn/高二/VCC/ui-shots';

(async () => {
  // argv: [states, prefix, page(index|overlay), w, h, dsf]
  const states = process.argv[2] ? process.argv[2].split(',') : ['listening', 'thinking', 'executing', 'idle'];
  const prefix = process.argv[3] || 'v2';
  const pageName = process.argv[4] || 'index';
  const w = parseInt(process.argv[5] || '420', 10);
  const h = parseInt(process.argv[6] || '720', 10);
  const dsf = parseFloat(process.argv[7] || '2');
  const BASE = `http://127.0.0.1:8137/ui/${pageName}.html`;
  const param = pageName === 'overlay' ? 'phase' : 'demo';

  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const page = await browser.newPage({
    viewport: { width: w, height: h },
    deviceScaleFactor: dsf,
  });
  for (const st of states) {
    const bg = pageName === 'overlay' ? '&bg=dark' : '';
    const bust = '&cb=' + Date.now(); // 绕过 Chromium 缓存
    await page.goto(`${BASE}?${param}=${st}${bg}${bust}`);
    await page.waitForTimeout(1400);
    await page.screenshot({ path: path.join(OUT, `${prefix}-${st}-t1.png`) });
    await page.waitForTimeout(1100);
    await page.screenshot({ path: path.join(OUT, `${prefix}-${st}-t2.png`) });
    console.log('shot:', `${prefix}-${st}-t1/t2.png`);
  }
  await browser.close();
})().catch((e) => { console.error(e); process.exit(1); });
