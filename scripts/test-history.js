/* 验证主窗输入历史（↑/↓）：demo 页面 → 输入两条 → Enter → ↑ 翻回 */
const { chromium } = require('playwright-core');

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const page = await browser.newPage({ viewport: { width: 420, height: 720 } });
  const errs = [];
  page.on('pageerror', (e) => errs.push(e.message));
  await page.goto('http://127.0.0.1:8137/ui/index.html?demo=none&cb=' + Date.now());
  await page.waitForTimeout(800);

  const input = page.locator('#input');
  await input.click();
  await input.fill('把音量调到 30');
  await input.press('Enter');
  await input.fill('打开课件文件夹');
  await input.press('Enter');
  await page.waitForTimeout(300);

  // ↑ 一次 → 应为「打开课件文件夹」；再 ↑ → 「把音量调到 30」；↓ → 回「打开课件文件夹」；再 ↓ → 草稿(空)
  await input.press('ArrowUp');
  const v1 = await input.inputValue();
  await input.press('ArrowUp');
  const v2 = await input.inputValue();
  await input.press('ArrowDown');
  const v3 = await input.inputValue();
  await input.press('ArrowDown');
  const v4 = await input.inputValue();

  console.log(JSON.stringify({ v1, v2, v3, v4, errs }, null, 1));
  const ok = v1 === '打开课件文件夹' && v2 === '把音量调到 30' && v3 === '打开课件文件夹' && v4 === '' && errs.length === 0;
  console.log(ok ? 'PASS' : 'FAIL');
  await browser.close();
  process.exit(ok ? 0 : 1);
})().catch((e) => { console.error(e); process.exit(1); });
