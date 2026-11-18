import { expect, test } from '@playwright/test';

const MARKETPLACE = 'https://knittingcode.github.io/deckpilot-plugins';

test.describe('language', () => {
  test('switching keeps the same page, in both directions', async ({ page }) => {
    for (const route of ['', 'download/', 'screenshots/', 'docs/', 'docs/workflows/']) {
      await page.goto(`en/${route}`);
      await page.getByRole('link', { name: /Polski/ }).click();
      await expect(page).toHaveURL(new RegExp(`/pl/${route}$`));
      await expect(page.locator('html')).toHaveAttribute('lang', 'pl');
      await page.getByRole('link', { name: /English/ }).click();
      await expect(page).toHaveURL(new RegExp(`/en/${route}$`));
      await expect(page.locator('html')).toHaveAttribute('lang', 'en');
    }
  });

  test('the root forwards a Polish browser to /pl/ and others to /en/', async ({ browser }) => {
    for (const [locale, expected] of [['pl-PL', '/pl/'], ['de-DE', '/en/'], ['en-US', '/en/']] as const) {
      const context = await browser.newContext({ locale });
      const page = await context.newPage();
      await page.goto('./');
      await expect(page).toHaveURL(new RegExp(`${expected}$`));
      await context.close();
    }
  });

  test('the root remembers a chosen language', async ({ browser }) => {
    const context = await browser.newContext({ locale: 'en-US' });
    const page = await context.newPage();
    await page.goto('en/');
    await page.getByRole('link', { name: /Polski/ }).click();
    await expect(page).toHaveURL(/\/pl\/$/);
    await page.goto('./');
    await expect(page).toHaveURL(/\/pl\/$/);
    await context.close();
  });

  test('a page never mixes languages in its interface', async ({ page }) => {
    await page.goto('pl/');
    const header = page.locator('header.site-header');
    await expect(header.getByRole('link', { name: 'Pobierz', exact: true })).toBeVisible();
    await expect(header.getByRole('link', { name: 'Download', exact: true })).toHaveCount(0);
    await expect(page.locator('footer')).toContainText('Dokumentacja użytkownika');
    await expect(page.locator('footer')).not.toContainText('User documentation');
  });
});

test.describe('theme', () => {
  test('follows the system, toggles, and persists the choice', async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.goto('en/');
    const background = () => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    expect(await background()).toBe('rgb(7, 17, 47)');
    await page.getByRole('button', { name: 'Switch to light theme' }).click();
    expect(await background()).toBe('rgb(255, 255, 255)');
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
    await page.reload();
    expect(await background()).toBe('rgb(255, 255, 255)');
    await page.getByRole('button', { name: 'Switch to dark theme' }).click();
    expect(await background()).toBe('rgb(7, 17, 47)');
    await page.goto('pl/download/');
    expect(await background()).toBe('rgb(7, 17, 47)');
  });

  test('is light for a light system without a stored choice', async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'light' });
    await page.goto('en/');
    expect(await page.evaluate(() => getComputedStyle(document.body).backgroundColor)).toBe('rgb(255, 255, 255)');
  });
});

test.describe('navigation', () => {
  test('desktop: every main link leads to its page and is marked current', async ({ page }) => {
    await page.goto('en/');
    const nav = page.getByRole('navigation', { name: 'Main' }).first();
    for (const [name, route, heading] of [
      ['Download', 'download', 'Download DeckPilot'],
      ['Docs', 'docs', 'Documentation'],
      ['Screenshots', 'screenshots', 'Screenshots'],
    ] as const) {
      await nav.getByRole('link', { name, exact: true }).click();
      await expect(page).toHaveURL(new RegExp(`/en/${route}/$`));
      await expect(page.getByRole('heading', { level: 1 })).toHaveText(heading);
      await expect(nav.getByRole('link', { name, exact: true })).toHaveAttribute('aria-current', 'page');
    }
  });

  test('mobile: the menu opens, navigates, and closes with Escape', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto('pl/');
    const toggle = page.getByRole('button', { name: 'Otwórz menu' });
    await expect(page.locator('#mobile-nav')).toBeHidden();
    await toggle.click();
    await expect(page.locator('#mobile-nav')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Zamknij menu' })).toHaveAttribute('aria-expanded', 'true');
    await page.keyboard.press('Escape');
    await expect(page.locator('#mobile-nav')).toBeHidden();
    await toggle.click();
    await page.locator('#mobile-nav').getByRole('link', { name: 'Dokumentacja', exact: true }).click();
    await expect(page).toHaveURL(/\/pl\/docs\/$/);
  });

  test('mobile: no page scrolls sideways', async ({ page }) => {
    await page.setViewportSize({ width: 360, height: 800 });
    for (const path of ['en/', 'pl/', 'pl/download/', 'pl/screenshots/', 'pl/docs/', 'pl/docs/layouts/', 'en/docs/actions/', 'pl/docs/installation/']) {
      await page.goto(path);
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      expect(overflow, path).toBeLessThanOrEqual(0);
    }
  });

  test('keyboard: skip link is the first stop and moves to the content', async ({ page }) => {
    await page.goto('en/');
    await page.keyboard.press('Tab');
    const skip = page.getByRole('link', { name: 'Skip to content' });
    await expect(skip).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(page.locator('#main')).toBeFocused();
  });

  test('keyboard: focus is visible on interactive elements', async ({ page }) => {
    await page.goto('en/');
    for (let i = 0; i < 6; i++) await page.keyboard.press('Tab');
    const outline = await page.evaluate(() => {
      const el = document.activeElement as HTMLElement;
      const style = getComputedStyle(el);
      return { tag: el.tagName, width: style.outlineWidth, style: style.outlineStyle };
    });
    expect(outline.style).toBe('solid');
    expect(parseFloat(outline.width)).toBeGreaterThanOrEqual(2);
  });

  test('keyboard: FAQ items open with Enter', async ({ page }) => {
    await page.goto('en/#faq');
    const first = page.locator('.faq summary').first();
    await first.focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('.faq details').first()).toHaveAttribute('open', '');
  });
});

test.describe('Marketplace', () => {
  test('links lead to the Marketplace site in the page language', async ({ page }) => {
    for (const lang of ['en', 'pl'] as const) {
      await page.goto(`${lang}/`);
      const links = page.locator('a[data-marketplace-link]');
      expect(await links.count()).toBeGreaterThanOrEqual(5);
      const allowed = [`${MARKETPLACE}/${lang}/`, `${MARKETPLACE}/${lang}/developers/`];
      for (const href of await links.evaluateAll((els) => els.map((el) => (el as HTMLAnchorElement).href))) {
        expect(allowed).toContain(href);
      }
      const nav = page.getByRole('navigation', { name: lang === 'en' ? 'Main' : 'Główna' }).first();
      await expect(nav.locator('a[data-marketplace-link]')).toHaveAttribute('href', `${MARKETPLACE}/${lang}/`);
    }
  });

  test('the Marketplace pages that are linked exist', async ({ request }) => {
    for (const path of ['en/', 'pl/', 'en/developers/', 'pl/developers/']) {
      expect((await request.get(`${MARKETPLACE}/${path}`)).status(), path).toBe(200);
    }
  });

  test('the homepage says that plugins ship with DeckPilot', async ({ page }) => {
    await page.goto('en/');
    await expect(page.locator('#marketplace')).toContainText('New');
    await expect(page.locator('#marketplace')).toContainText('installs plugins from Settings › Extensions › Plugins');
    await expect(page.locator('#marketplace')).not.toContainText('switched off');
  });
});

test.describe('404', () => {
  test('a missing page answers 404 with the English page', async ({ page }) => {
    const response = await page.goto('en/no-such-page/');
    expect(response?.status()).toBe(404);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('This page does not exist');
    await page.getByRole('link', { name: 'Back to the homepage' }).click();
    await expect(page).toHaveURL(/\/en\/$/);
  });

  test('a missing Polish page ends on the Polish 404 page', async ({ page }) => {
    const response = await page.goto('pl/nie-ma-takiej-strony/');
    expect(response?.status()).toBe(404);
    await expect(page).toHaveURL(/\/pl\/404\/$/);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Ta strona nie istnieje');
    await expect(page.locator('html')).toHaveAttribute('lang', 'pl');
  });

  test('nothing that an updater could read is served', async ({ request }) => {
    for (const path of ['latest.json', 'prerelease.json', 'release-meta.json', 'SHA256SUMS', 'releases/latest/download/latest.json']) {
      expect((await request.get(path, { failOnStatusCode: false })).status(), path).toBe(404);
    }
  });
});
