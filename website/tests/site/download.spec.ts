import { expect, test, type APIRequestContext } from '@playwright/test';

/**
 * The download page against GitHub itself: what the page shows must be the
 * latest published release, and every button must lead to a file that
 * exists. These tests use the network (api.github.com, github.com).
 */
const API = 'https://api.github.com/repos/knittingcode/deckpilot/releases?per_page=30';
const PREFIX = 'https://github.com/knittingcode/deckpilot/releases/download/';

type Asset = { name: string; size: number; browser_download_url: string; digest?: string | null };
type Release = { tag_name: string; draft: boolean; prerelease: boolean; published_at: string | null; html_url: string; assets: Asset[] };

async function latestPublished(request: APIRequestContext): Promise<Release | undefined> {
  const token = process.env.GITHUB_TOKEN;
  const response = await request.get(API, {
    headers: { Accept: 'application/vnd.github+json', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
  });
  expect(response.status(), 'GitHub API').toBe(200);
  const published = ((await response.json()) as Release[])
    .filter((release) => !release.draft && release.published_at)
    .sort((a, b) => Date.parse(b.published_at!) - Date.parse(a.published_at!));
  return published.find((release) => !release.prerelease) ?? published[0];
}

const isInstaller = (name: string) => /(\.dmg|-setup\.exe|\.msi|\.AppImage|\.deb|\.rpm)$/.test(name);

test('shows exactly the latest published release of the GitHub API', async ({ page, request }) => {
  const release = await latestPublished(request);
  await page.goto('en/download/');
  const root = page.locator('[data-download]');
  if (!release) {
    await expect(root).toHaveAttribute('data-release-tag', '');
    await expect(page.locator('#no-release')).toBeVisible();
    return;
  }
  await expect(root).toHaveAttribute('data-release-tag', release.tag_name);
  await expect(page.locator('[data-release-version]')).toHaveText(release.tag_name);
  await expect(page.locator('[data-release-channel]')).toHaveText(release.prerelease ? 'Pre-release' : 'Stable release');
  await expect(page.locator('[data-release-date]')).toHaveAttribute('datetime', release.published_at!);
  await expect(page.locator('[data-release-notes]')).toHaveAttribute('href', release.html_url);

  const expected = release.assets.filter((asset) => isInstaller(asset.name)).map((asset) => asset.name).sort();
  const shown = await page.locator('[data-asset]').evaluateAll((els) => els.map((el) => el.getAttribute('data-asset') ?? '').sort());
  expect(shown).toEqual(expected);

  for (const asset of release.assets.filter((a) => isInstaller(a.name))) {
    const card = page.locator(`[data-asset="${asset.name}"]`);
    await expect(card.locator('[data-asset-link]')).toHaveAttribute('href', asset.browser_download_url);
    const digest = /^sha256:([0-9a-f]{64})$/.exec(asset.digest ?? '')?.[1];
    if (digest) await expect(card.locator('[data-asset-sha]')).toHaveText(digest);
    await expect(card).toContainText(`${(asset.size / 1048576).toFixed(1)} MB`);
  }
});

test('never shows a draft or a file from another place', async ({ page }) => {
  await page.goto('en/download/');
  const links = await page.locator('[data-asset-link]').evaluateAll((els) => els.map((el) => (el as HTMLAnchorElement).href));
  for (const href of links) {
    expect(href.toLowerCase().startsWith(PREFIX)).toBe(true);
    expect(href).not.toContain('untagged-');
  }
  // No installer and no update file is hosted by this site.
  const own = await page.locator('main a[href]').evaluateAll((els) =>
    els.map((el) => (el as HTMLAnchorElement).href).filter((href) => /\.(dmg|exe|msi|AppImage|deb|rpm|sig|tar\.gz|json)$/i.test(new URL(href).pathname)),
  );
  for (const href of own) expect(new URL(href).hostname).toBe('github.com');
});

test('every download button leads to a file that exists', async ({ page, request }) => {
  test.setTimeout(120_000);
  await page.goto('en/download/');
  const links = await page.locator('[data-asset-link], [data-checksums]').evaluateAll((els) => els.map((el) => (el as HTMLAnchorElement).href));
  for (const href of links) {
    const response = await request.fetch(href, { method: 'HEAD', maxRedirects: 5 });
    expect(response.status(), href).toBe(200);
    expect(Number(response.headers()['content-length'] ?? 0), href).toBeGreaterThan(0);
  }
});

test('the checksums on the page are those of the release SHA256SUMS file', async ({ page, request }) => {
  await page.goto('en/download/');
  const sums = page.locator('[data-checksums]');
  test.skip((await sums.count()) === 0, 'the release has no SHA256SUMS file');
  const text = await (await request.get((await sums.getAttribute('href'))!)).text();
  const listed = new Map(text.trim().split('\n').map((line) => line.trim().split(/\s+/).reverse() as [string, string]));
  for (const card of await page.locator('[data-asset]').all()) {
    const name = (await card.getAttribute('data-asset'))!;
    const sha = card.locator('[data-asset-sha]');
    if (await sha.count()) expect(await sha.textContent(), name).toBe(listed.get(name));
  }
});

test('signing statements match what the release records', async ({ page, request }) => {
  await page.goto('en/download/');
  const tag = await page.locator('[data-download]').getAttribute('data-release-tag');
  test.skip(!tag, 'no published release');
  const response = await request.get(`${PREFIX}${tag}/release-meta.json`, { failOnStatusCode: false });
  test.skip(response.status() !== 200, 'the release has no release-meta.json');
  const platforms = ((await response.json()) as { platforms: Record<string, { codesign: string; notarized: boolean | null }> }).platforms;
  const statement = (os: string) => page.locator(`[data-os="${os}"] [data-asset-signing]`).first();
  const mac = platforms['aarch64-apple-darwin'];
  if (mac?.codesign === 'developer-id' && mac.notarized) await expect(statement('macos')).toContainText('notarized by Apple');
  const windows = platforms['x86_64-pc-windows-msvc'];
  if (windows?.codesign === 'unsigned') {
    await expect(statement('windows')).toContainText('Not code-signed');
    await expect(statement('windows')).toContainText('SmartScreen');
  }
  if (windows?.codesign === 'authenticode') await expect(statement('windows')).toContainText('Authenticode');
  const linux = platforms['x86_64-unknown-linux-gnu'];
  if (linux?.codesign === 'unsigned') await expect(statement('linux')).toContainText('not signed');
});

test('both languages show the same release', async ({ page }) => {
  await page.goto('en/download/');
  const en = await page.locator('[data-asset-link]').evaluateAll((els) => els.map((el) => (el as HTMLAnchorElement).href));
  const tag = await page.locator('[data-download]').getAttribute('data-release-tag');
  await page.goto('pl/download/');
  await expect(page.locator('[data-download]')).toHaveAttribute('data-release-tag', tag ?? '');
  expect(await page.locator('[data-asset-link]').evaluateAll((els) => els.map((el) => (el as HTMLAnchorElement).href))).toEqual(en);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Pobierz DeckPilota');
});
