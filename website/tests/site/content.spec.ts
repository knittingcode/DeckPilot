import { expect, test } from '@playwright/test';

test.describe('homepage', () => {
  test('has the sections in order, in both languages', async ({ page }) => {
    await page.goto('en/');
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Your Stream Deck. Your workflow.');
    const ids = await page
      .locator('main > section[id], main > section[aria-labelledby="hero-title"]')
      .evaluateAll((els) => els.map((el) => el.id || 'hero'));
    expect(ids).toEqual(['hero', 'overview', 'platforms', 'devices', 'layouts', 'actions', 'marketplace', 'agent', 'security', 'open-source', 'faq']);
    await page.goto('pl/');
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Twój Stream Deck. Twój sposób pracy.');
  });

  test('hero actions lead to the download page, the Marketplace and GitHub', async ({ page }) => {
    await page.goto('en/');
    const hero = page.locator('section.hero');
    await expect(hero.getByRole('link', { name: 'Download DeckPilot' })).toHaveAttribute('href', /\/deckpilot\/en\/download\/$/);
    await expect(hero.getByRole('link', { name: 'Explore Plugins' })).toHaveAttribute('href', 'https://knittingcode.github.io/deckpilot-plugins/en/');
    await expect(hero.getByRole('link', { name: 'View on GitHub' })).toHaveAttribute('href', 'https://github.com/knittingcode/deckpilot');
    await page.goto('pl/');
    await expect(page.locator('section.hero').getByRole('link', { name: 'Pobierz DeckPilota' })).toHaveAttribute('href', /\/deckpilot\/pl\/download\/$/);
  });

  test('is plain about what is supported', async ({ page }) => {
    await page.goto('en/');
    const devices = page.locator('#devices');
    await expect(devices.getByRole('row', { name: /Stream Deck MK\.2/ })).toContainText('Supported');
    await expect(devices.getByRole('row', { name: /Stream Deck XL/ })).toContainText('Layout design only');
    await expect(devices.getByRole('row', { name: /Stream Deck \+/ })).toContainText('No driver');
    await expect(page.locator('#platforms')).toContainText('Not code-signed yet');
    await expect(page.locator('#security')).toContainText('has not had an independent security review');
    await expect(page.locator('footer')).toContainText('not affiliated with or endorsed by Elgato');
    await expect(page.locator('footer')).toContainText('Apache-2.0');
  });

  test('the release badge names the release the download page offers', async ({ page }) => {
    await page.goto('en/download/');
    const tag = await page.locator('[data-download]').getAttribute('data-release-tag');
    await page.goto('en/');
    const badge = page.locator('[data-release-badge]');
    if (tag) await expect(badge).toContainText(tag);
    else await expect(badge).toHaveText('In development');
  });
});

test.describe('documentation', () => {
  const PAGES = [
    'Installation',
    'First launch',
    'Devices',
    'Profiles and projects',
    'Layouts',
    'Gestures',
    'Actions',
    'Workflows',
    'Variables',
    'Background agent',
    'Updates',
    'Plugins',
    'Troubleshooting',
  ];

  test('sidebar lists every page in order and marks the current one', async ({ page }) => {
    await page.goto('en/docs/workflows/');
    const sidebar = page.getByRole('navigation', { name: 'Documentation pages' });
    await expect(sidebar.getByRole('link')).toHaveText(PAGES);
    await expect(sidebar.getByRole('link', { name: 'Workflows' })).toHaveAttribute('aria-current', 'page');
    await sidebar.getByRole('link', { name: 'Variables' }).click();
    await expect(page).toHaveURL(/\/en\/docs\/variables\/$/);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Variables');
  });

  test('the index links every page, in both languages', async ({ page }) => {
    await page.goto('en/docs/');
    await expect(page.locator('main .card-title')).toHaveText(PAGES);
    await page.goto('pl/docs/');
    await expect(page.locator('main .card-title')).toHaveCount(PAGES.length);
    await expect(page.locator('main .card-title').first()).toHaveText('Instalacja');
  });

  test('previous and next links walk the pages in order', async ({ page }) => {
    await page.goto('en/docs/installation/');
    const pager = page.getByRole('navigation', { name: 'Previous and next page' });
    await pager.getByRole('link').click();
    await expect(page).toHaveURL(/\/en\/docs\/first-launch\/$/);
    await pager.getByRole('link', { name: /Next/ }).click();
    await expect(page).toHaveURL(/\/en\/docs\/devices\/$/);
    await pager.getByRole('link', { name: /Previous/ }).click();
    await expect(page).toHaveURL(/\/en\/docs\/first-launch\/$/);
  });

  test('the table of contents jumps to a heading', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('en/docs/installation/');
    await page.getByRole('navigation', { name: 'On this page' }).getByRole('link', { name: 'Linux' }).click();
    await expect(page).toHaveURL(/#linux$/);
    await expect(page.locator('#linux')).toBeInViewport();
  });

  test('code can be copied', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.goto('en/docs/installation/');
    const block = page.locator('.prose .code-block').first();
    await block.hover();
    await block.getByRole('button', { name: 'Copy code' }).click();
    await expect(block.getByRole('button')).toHaveText('Copied');
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(await block.locator('pre').textContent());
  });

  test('plugins are documented as installable from the Marketplace', async ({ page }) => {
    await page.goto('en/docs/plugins/');
    await expect(page.locator('.prose')).toContainText('DeckPilot installs plugins from its signed Marketplace');
    await expect(page.locator('.prose').getByRole('link', { name: 'plugin developer portal' })).toHaveAttribute(
      'href',
      'https://knittingcode.github.io/deckpilot-plugins/en/developers/',
    );
    await page.goto('pl/docs/plugins/');
    await expect(page.locator('.prose').getByRole('link', { name: 'portal dla twórców wtyczek' })).toHaveAttribute(
      'href',
      'https://knittingcode.github.io/deckpilot-plugins/pl/developers/',
    );
  });

  test('the documentation sends plugin developers to the Marketplace site', async ({ page }) => {
    await page.goto('en/docs/');
    await expect(page.getByRole('link', { name: 'Open the plugin developer portal' })).toHaveAttribute(
      'href',
      'https://knittingcode.github.io/deckpilot-plugins/en/developers/',
    );
  });
});

test.describe('screenshots', () => {
  test('every frame is a real image or says that it is a placeholder', async ({ page }) => {
    for (const [lang, label] of [['en', 'Screenshot not available yet'], ['pl', 'Zrzut ekranu nie jest jeszcze dostępny']] as const) {
      await page.goto(`${lang}/screenshots/`);
      const shots = page.locator('figure[data-screenshot]');
      await expect(shots).toHaveCount(7);
      for (const shot of await shots.all()) {
        if ((await shot.getAttribute('data-state')) === 'image') {
          const image = shot.locator('img').first();
          expect((await image.getAttribute('alt'))?.length ?? 0).toBeGreaterThan(20);
          expect(await image.evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth > 0)).toBe(true);
        } else {
          await expect(shot.locator('img')).toHaveCount(0);
          await expect(shot.locator('.shot-frame')).toHaveText(label);
        }
        await expect(shot.locator('figcaption')).not.toBeEmpty();
      }
    }
  });
});
