// Renders src/assets/og.png (1200x630), the image shown when a page of the
// site is shared. Run by hand when the artwork should change:
//
//   node tools/make-og.mjs
//
// It composes the unmodified DeckPilot wordmark on the brand navy with the
// site's headline; the brand files themselves are never altered.
import { chromium } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const wordmark = readFileSync(join(root, 'src/assets/brand/wordmark-dark.svg')).toString('base64');
const font = readFileSync(join(root, 'node_modules/@fontsource-variable/inter/files/inter-latin-wght-normal.woff2')).toString('base64');

const html = `<!doctype html><html><head><style>
@font-face { font-family: Inter; src: url(data:font/woff2;base64,${font}) format('woff2'); font-weight: 100 900; }
* { box-sizing: border-box; margin: 0; }
body { width: 1200px; height: 630px; background: #07112f; color: #eef2fb; font-family: Inter, sans-serif; position: relative; overflow: hidden; }
.glow { position: absolute; inset: 0; background: radial-gradient(60% 70% at 78% 10%, rgb(32 165 255 / 0.28), transparent 70%),
  linear-gradient(rgb(255 255 255 / 0.035) 1px, transparent 1px) 0 0 / 100% 48px, linear-gradient(90deg, rgb(255 255 255 / 0.035) 1px, transparent 1px) 0 0 / 48px 100%; }
.content { position: absolute; inset: 0; padding: 80px 88px; display: flex; flex-direction: column; justify-content: space-between; }
img { height: 64px; align-self: flex-start; }
h1 { font-size: 76px; line-height: 1.04; font-weight: 700; letter-spacing: -0.035em; max-width: 900px; }
p { margin-top: 28px; font-size: 30px; color: #b1bcd9; letter-spacing: -0.01em; }
.bar { position: absolute; left: 0; right: 0; bottom: 0; height: 10px; background: #20a5ff; }
</style></head><body><div class="glow"></div><div class="content">
<img src="data:image/svg+xml;base64,${wordmark}" alt="">
<div><h1>Your Stream Deck.<br>Your workflow.</h1><p>The open-source Stream Deck controller and automation app</p></div>
</div><div class="bar"></div></body></html>`;

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
await page.setContent(html, { waitUntil: 'networkidle' });
await page.evaluate(() => document.fonts.ready);
await page.screenshot({ path: join(root, 'src/assets/og.png'), type: 'png' });
await browser.close();
console.log('wrote src/assets/og.png');
