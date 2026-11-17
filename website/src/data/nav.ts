import type { Dictionary } from '@/i18n';

/**
 * Main navigation. A `route` is a page of this site (after the language);
 * `marketplace` is a page of the Plugin Marketplace site, in the same
 * language.
 */
export type NavItem = { key: keyof Dictionary['nav'] } & ({ route: string } | { marketplace: string });

export const NAV: NavItem[] = [
  { route: 'download', key: 'download' },
  { route: 'docs', key: 'docs' },
  { route: 'screenshots', key: 'screenshots' },
  { marketplace: '', key: 'plugins' },
];
