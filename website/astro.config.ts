import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';

// The production URL is the application repository's GitHub Pages site.
// SITE_OUT_DIR lets the tests build variants (tests/build-fixtures.mjs)
// next to the real build without touching it.
const site = process.env.SITE_ORIGIN ?? 'https://knittingcode.github.io';
const base = process.env.SITE_BASE ?? '/deckpilot';

export default defineConfig({
  site,
  base,
  trailingSlash: 'always',
  outDir: process.env.SITE_OUT_DIR ?? './dist',
  build: { format: 'directory', assets: '_astro' },
  integrations: [
    sitemap({
      // The root page only forwards to a language; 404 is not a destination.
      filter: (page) => !/\/404\/?$/.test(page) && new URL(page).pathname !== `${base}/`,
      i18n: { defaultLocale: 'en', locales: { en: 'en', pl: 'pl' } },
    }),
  ],
  markdown: {
    shikiConfig: {
      themes: { light: 'github-light-high-contrast', dark: 'github-dark-default' },
      defaultColor: false,
      wrap: false,
    },
  },
  vite: { plugins: [tailwindcss()] },
  devToolbar: { enabled: false },
});
