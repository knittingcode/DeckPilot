/**
 * Addresses the pages link to. Nothing here names a DeckPilot version: the
 * release shown on the site is read from GitHub at build time
 * (src/lib/releases.ts).
 */

export const REPO_SLUG = 'knittingcode/deckpilot';
export const REPO_APP = `https://github.com/${REPO_SLUG}`;
export const REPO_PLUGINS = 'https://github.com/knittingcode/deckpilot-plugins';

/** The Plugin Marketplace: a separate site with its own repository. */
export const MARKETPLACE = 'https://knittingcode.github.io/deckpilot-plugins';

/** A Marketplace page in the visitor's language: marketplace('pl', 'docs'). */
export function marketplace(lang: 'en' | 'pl', path = ''): string {
  const clean = path.replace(/^\/+|\/+$/g, '');
  return `${MARKETPLACE}/${lang}/${clean ? `${clean}/` : ''}`;
}

const blob = (path: string) => `${REPO_APP}/blob/main/${path}`;

export const LINKS = {
  app: REPO_APP,
  releases: `${REPO_APP}/releases`,
  issues: `${REPO_APP}/issues`,
  newIssue: `${REPO_APP}/issues/new/choose`,
  reportVulnerability: `${REPO_APP}/security/advisories/new`,
  readme: `${REPO_APP}#readme`,
  changelog: blob('CHANGELOG.md'),
  contributing: blob('CONTRIBUTING.md'),
  codeOfConduct: blob('CODE_OF_CONDUCT.md'),
  security: blob('SECURITY.md'),
  license: blob('LICENSE'),
  roadmap: blob('docs/roadmap.md'),
  docsSecurity: blob('docs/security.md'),
  docsDevices: blob('docs/devices.md'),
  docsUpdates: blob('docs/updates.md'),
  docsDevelopment: blob('docs/development.md'),
  docsArchitecture: blob('docs/architecture.md'),
  docsIndex: blob('docs/README.md'),
  udevRule: blob('packaging/linux/70-deckpilot-streamdeck.rules'),
  udevRuleRaw: `https://raw.githubusercontent.com/${REPO_SLUG}/main/packaging/linux/70-deckpilot-streamdeck.rules`,
  plugins: REPO_PLUGINS,
} as const;
