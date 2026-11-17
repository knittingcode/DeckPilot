import { defineConfig, devices } from '@playwright/test';

/**
 * Browser tests run against static builds, served the way GitHub Pages
 * serves them (tests/serve.mjs):
 *   site     dist/                  the real build (releases read from GitHub)
 *   none     dist-fixtures/none/    no published release (never deployed)
 *   stable   dist-fixtures/stable/  a stable release, a draft, ... (never deployed)
 * Build them first: `npm run build && npm run build:fixtures`.
 */
const BASE = '/deckpilot';
// SITE_URL=https://knittingcode.github.io/deckpilot/ runs the `site` suite
// against a deployed site instead of the local build (npm run test:live).
const live = process.env.SITE_URL?.replace(/\/?$/, '/');
const local = (port: number) => `http://127.0.0.1:${port}${BASE}/`;
const server = (dir: string, port: number) => ({
  command: `node tests/serve.mjs ${dir} ${port}`,
  url: `${local(port)}en/`,
  reuseExistingServer: !process.env.CI,
});

export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI || live ? 1 : 0,
  reporter: process.env.CI ? [['github'], ['list']] : 'list',
  use: { ...devices['Desktop Chrome'], trace: 'retain-on-failure' },
  projects: live
    ? [{ name: 'site', testDir: './tests/site', use: { baseURL: live } }]
    : [
        { name: 'site', testDir: './tests/site', use: { baseURL: local(4321) } },
        { name: 'none', testDir: './tests/fixture', testMatch: 'none.spec.ts', use: { baseURL: local(4322) } },
        { name: 'stable', testDir: './tests/fixture', testMatch: 'stable.spec.ts', use: { baseURL: local(4323) } },
      ],
  webServer: live ? undefined : [server('dist', 4321), server('dist-fixtures/none', 4322), server('dist-fixtures/stable', 4323)],
});
