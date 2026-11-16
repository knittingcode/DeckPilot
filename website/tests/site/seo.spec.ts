import { expect, test } from '@playwright/test';

const ORIGIN = 'https://knittingcode.github.io/deckpilot';
const ROUTES = ['', 'download/', 'screenshots/', 'docs/', 'docs/installation/', 'docs/troubleshooting/'];

for (const lang of ['en', 'pl'] as const) {
  for (const route of ROUTES) {
    test(`metadata: ${lang}/${route}`, async ({ page }) => {
      await page.goto(`${lang}/${route}`);
      await expect(page.locator('html')).toHaveAttribute('lang', lang);
      const title = await page.title();
      expect(title.length).toBeGreaterThan(10);
      expect(title.length).toBeLessThanOrEqual(70);
      const description = await page.locator('meta[name="description"]').getAttribute('content');
      expect(description?.length ?? 0).toBeGreaterThan(50);
      expect(description?.length ?? 0).toBeLessThanOrEqual(230);
      await expect(page.locator('link[rel="canonical"]')).toHaveAttribute('href', `${ORIGIN}/${lang}/${route}`);
      await expect(page.locator('link[rel="alternate"][hreflang="en"]')).toHaveAttribute('href', `${ORIGIN}/en/${route}`);
      await expect(page.locator('link[rel="alternate"][hreflang="pl"]')).toHaveAttribute('href', `${ORIGIN}/pl/${route}`);
      await expect(page.locator('link[rel="alternate"][hreflang="x-default"]')).toHaveAttribute('href', `${ORIGIN}/en/${route}`);
      await expect(page.locator('meta[property="og:title"]')).toHaveAttribute('content', title);
      await expect(page.locator('meta[property="og:description"]')).toHaveAttribute('content', description ?? '');
      await expect(page.locator('meta[property="og:url"]')).toHaveAttribute('content', `${ORIGIN}/${lang}/${route}`);
      await expect(page.locator('meta[property="og:locale"]')).toHaveAttribute('content', lang === 'en' ? 'en_US' : 'pl_PL');
      await expect(page.locator('meta[property="og:image"]')).toHaveAttribute('content', new RegExp(`^${ORIGIN}/_astro/.+\\.png$`));
      await expect(page.locator('meta[name="twitter:card"]')).toHaveAttribute('content', 'summary_large_image');
      await expect(page.locator('meta[name="robots"]')).toHaveCount(0);
      await expect(page.locator('meta[name="generator"]')).toHaveCount(0);
      await expect(page.getByRole('heading', { level: 1 })).toHaveCount(1);
    });
  }
}

test('titles and descriptions differ between languages', async ({ page }) => {
  for (const route of ROUTES) {
    await page.goto(`en/${route}`);
    const en = [await page.title(), await page.locator('meta[name="description"]').getAttribute('content')];
    await page.goto(`pl/${route}`);
    const pl = [await page.title(), await page.locator('meta[name="description"]').getAttribute('content')];
    expect(pl[0], route).not.toBe(en[0]);
    expect(pl[1], route).not.toBe(en[1]);
  }
});

test('heading levels never skip', async ({ page }) => {
  for (const path of ['en/', 'en/download/', 'en/screenshots/', 'en/docs/', 'en/docs/installation/', 'pl/docs/actions/']) {
    await page.goto(path);
    const levels = await page.locator('h1, h2, h3, h4, h5, h6').evaluateAll((els) => els.map((el) => Number(el.tagName[1])));
    for (let i = 1; i < levels.length; i++) {
      expect(levels[i]! - levels[i - 1]!, `${path}: h${levels[i - 1]} followed by h${levels[i]}`).toBeLessThanOrEqual(1);
    }
  }
});

test('the open graph image exists and is 1200 by 630', async ({ page, request }) => {
  await page.goto('en/');
  const url = await page.locator('meta[property="og:image"]').getAttribute('content');
  const local = new URL(url ?? '').pathname.replace(/^\/deckpilot\//, '');
  const response = await request.get(local);
  expect(response.status()).toBe(200);
  expect(response.headers()['content-type']).toContain('image/png');
  const bytes = await response.body();
  expect(bytes.readUInt32BE(16)).toBe(1200);
  expect(bytes.readUInt32BE(20)).toBe(630);
});

test('sitemap lists the pages of both languages and nothing else', async ({ request }) => {
  const index = await (await request.get('sitemap-index.xml')).text();
  expect(index).toContain(`${ORIGIN}/sitemap-0.xml`);
  const sitemap = await (await request.get('sitemap-0.xml')).text();
  const urls = [...sitemap.matchAll(/<loc>([^<]+)<\/loc>/g)].map((m) => m[1]!);
  expect(urls).toContain(`${ORIGIN}/en/`);
  expect(urls).toContain(`${ORIGIN}/en/download/`);
  expect(urls).toContain(`${ORIGIN}/pl/docs/workflows/`);
  expect(urls.filter((u) => u.includes('/en/')).length).toBe(urls.filter((u) => u.includes('/pl/')).length);
  for (const url of urls) {
    expect(url).toMatch(new RegExp(`^${ORIGIN}/(en|pl)/`));
    expect(url).not.toContain('/404/');
  }
  expect(sitemap).toContain('hreflang="pl"');
});

test('robots.txt allows crawling and names the sitemap', async ({ request }) => {
  const robots = await (await request.get('robots.txt')).text();
  expect(robots).toContain('Allow: /');
  expect(robots).toContain(`Sitemap: ${ORIGIN}/sitemap-index.xml`);
});

test('the root and 404 pages are kept out of indexes', async ({ request }) => {
  for (const path of ['index.html', '404.html', 'en/404/', 'pl/404/']) {
    expect(await (await request.get(path, { failOnStatusCode: false })).text(), path).toMatch(/<meta name="robots" content="noindex/);
  }
});

test('assets load from under the base path, and nothing from another host', async ({ page }) => {
  const outside: string[] = [];
  const failed: string[] = [];
  page.on('request', (request) => {
    const url = new URL(request.url());
    const own = url.hostname === '127.0.0.1' || url.hostname === 'knittingcode.github.io';
    if (own && !url.pathname.startsWith('/deckpilot/')) outside.push(url.pathname);
    if (!own) outside.push(url.href);
  });
  page.on('response', (response) => {
    if (response.status() >= 400) failed.push(`${response.status()} ${response.url()}`);
  });
  for (const path of ['en/', 'pl/docs/workflows/', 'en/screenshots/', 'pl/docs/']) await page.goto(path, { waitUntil: 'networkidle' });
  expect(outside).toEqual([]);
  expect(failed).toEqual([]);
});

test('the download page contacts only GitHub besides this site', async ({ page }) => {
  const outside = new Set<string>();
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (url.hostname !== '127.0.0.1' && url.hostname !== 'knittingcode.github.io') outside.add(url.origin);
  });
  await page.goto('en/download/', { waitUntil: 'networkidle' });
  expect([...outside].filter((origin) => origin !== 'https://api.github.com')).toEqual([]);
});
