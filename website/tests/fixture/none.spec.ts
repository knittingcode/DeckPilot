import { expect, test } from '@playwright/test';
import { expectAccessible } from '../axe';

/** A build from an empty release list: the site must say so honestly. */
test('the download page says that nothing has been published', async ({ page }) => {
  await page.goto('en/download/');
  await expect(page.locator('[data-download]')).toHaveAttribute('data-release-tag', '');
  await expect(page.locator('#no-release')).toContainText('No release has been published yet');
  await expect(page.locator('[data-asset-link]')).toHaveCount(0);
  await expect(page.getByRole('link', { name: 'Releases on GitHub' })).toHaveAttribute('href', 'https://github.com/knittingcode/deckpilot/releases');
  await page.goto('pl/download/');
  await expect(page.locator('#no-release')).toContainText('Nie opublikowano jeszcze żadnego wydania');
});

test('the homepage names no version', async ({ page }) => {
  await page.goto('en/');
  await expect(page.locator('[data-release-badge]')).toHaveText('In development');
  expect(await page.locator('body').innerText()).not.toMatch(/\bv?\d+\.\d+\.\d+(-[a-z]+\.\d+)?\b/);
});

for (const scheme of ['light', 'dark'] as const) {
  test(`axe: empty download page (${scheme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
    await page.goto('en/download/');
    await expectAccessible(page);
  });
}
