import { test } from '@playwright/test';
import { expectAccessible } from '../axe';

const PAGES = [
  'en/',
  'pl/',
  'en/download/',
  'pl/download/',
  'en/screenshots/',
  'pl/screenshots/',
  'en/docs/',
  'pl/docs/',
  'en/docs/installation/',
  'pl/docs/workflows/',
  'en/docs/layouts/',
  'pl/docs/plugins/',
  'en/404/',
  'pl/404/',
];

for (const scheme of ['light', 'dark'] as const) {
  for (const path of PAGES) {
    test(`axe: ${path} (${scheme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
      await page.goto(path);
      await expectAccessible(page);
    });
  }
}

for (const scheme of ['light', 'dark'] as const) {
  test(`axe: mobile menu open (${scheme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto('en/');
    await page.getByRole('button', { name: 'Open menu' }).click();
    await expectAccessible(page);
  });

  test(`axe: FAQ answers open (${scheme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
    await page.goto('pl/');
    for (const summary of await page.locator('.faq summary').all()) await summary.click();
    await expectAccessible(page);
  });
}
