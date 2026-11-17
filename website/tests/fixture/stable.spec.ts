import { expect, test } from '@playwright/test';
import { expectAccessible } from '../axe';

/**
 * A build from tests/fixtures/releases-stable.json: a draft v9.9.9, a
 * pre-release v1.1.0-beta.1, a stable v1.0.0 with one asset hosted
 * elsewhere, and signing recorded for macOS and Windows only.
 */
test('offers the stable release and mentions the newer pre-release', async ({ page }) => {
  await page.goto('en/download/');
  await expect(page.locator('[data-download]')).toHaveAttribute('data-release-tag', 'v1.0.0');
  await expect(page.locator('[data-release-channel]')).toHaveText('Stable release');
  await expect(page.locator('main')).toContainText('A newer pre-release, v1.1.0-beta.1, is also available.');
  await expect(page.locator('main')).not.toContainText('This is a pre-release');
});

test('the draft appears nowhere on the site', async ({ page }) => {
  for (const path of ['en/', 'en/download/', 'pl/download/']) {
    await page.goto(path);
    const html = await page.content();
    expect(html, path).not.toContain('9.9.9');
    expect(html, path).not.toContain('untagged-draft');
  }
});

test('buttons exist only for installers of this repository', async ({ page }) => {
  await page.goto('en/download/');
  const names = await page.locator('[data-asset]').evaluateAll((els) => els.map((el) => el.getAttribute('data-asset')));
  expect(names).toEqual(['DeckPilot_1.0.0_aarch64.dmg', 'DeckPilot_1.0.0_x64-setup.exe', 'DeckPilot-1.0.0-1.x86_64.rpm']);
  expect(await page.content()).not.toContain('example.com');
  await expect(page.locator('[data-os="linux"]')).toContainText('RPM package (.rpm) · x64');
});

test('signing is stated only where the release records it', async ({ page }) => {
  await page.goto('en/download/');
  await expect(page.locator('[data-os="macos"] [data-asset-signing]')).toContainText('notarized by Apple');
  await expect(page.locator('[data-os="windows"] [data-asset-signing]')).toContainText('Code-signed (Authenticode)');
  await expect(page.locator('[data-os="windows"] [data-asset-signing]')).not.toContainText('SmartScreen');
  await expect(page.locator('[data-os="linux"] [data-asset-signing]')).toContainText('does not record how this build was signed');
  await expect(page.locator('[data-os="linux"]')).toContainText('No checksum is published for this file.');
});

test('the homepage badge shows the stable version', async ({ page }) => {
  await page.goto('pl/');
  await expect(page.locator('[data-release-badge]')).toHaveText('Wersja v1.0.0');
});

test('a newer release published after the build is announced', async ({ page }) => {
  await page.route('https://api.github.com/**', (route) =>
    route.fulfill({
      json: [
        { tag_name: 'v9.9.9', draft: true, prerelease: false, published_at: null, html_url: 'https://github.com/knittingcode/deckpilot/releases/tag/untagged' },
        { tag_name: 'v1.2.0-rc.1', draft: false, prerelease: true, published_at: '2030-05-01T00:00:00Z', html_url: 'https://github.com/knittingcode/deckpilot/releases/tag/v1.2.0-rc.1' },
        { tag_name: 'v1.1.0', draft: false, prerelease: false, published_at: '2030-04-01T00:00:00Z', html_url: 'https://github.com/knittingcode/deckpilot/releases/tag/v1.1.0' },
        { tag_name: 'v1.0.0', draft: false, prerelease: false, published_at: '2030-02-01T10:00:00Z', html_url: 'https://github.com/knittingcode/deckpilot/releases/tag/v1.0.0' },
      ],
    }),
  );
  await page.goto('en/download/');
  const notice = page.locator('[data-fresh]');
  await expect(notice).toBeVisible();
  await expect(notice).toContainText('A newer release, v1.1.0, has been published since this page was built.');
  await expect(notice.getByRole('link')).toHaveAttribute('href', 'https://github.com/knittingcode/deckpilot/releases/tag/v1.1.0');
});

test('no notice when GitHub cannot be reached or has nothing newer', async ({ page }) => {
  await page.route('https://api.github.com/**', (route) => route.abort());
  await page.goto('en/download/', { waitUntil: 'networkidle' });
  await expect(page.locator('[data-fresh]')).toBeHidden();
  await expect(page.locator('[data-asset-link]')).toHaveCount(3);
});

for (const scheme of ['light', 'dark'] as const) {
  test(`axe: stable download page (${scheme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
    await page.route('https://api.github.com/**', (route) => route.abort());
    await page.goto('en/download/');
    await expectAccessible(page);
  });
}
