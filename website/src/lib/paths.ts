import type { Lang } from '@/i18n';

/** The site's base path without a trailing slash ('' at a domain root). */
export const BASE = import.meta.env.BASE_URL.replace(/\/$/, '');

/** An absolute path inside the site: asset('/v1/x') -> '/deckpilot-plugins/v1/x'. */
export function withBase(path: string): string {
  return `${BASE}${path.startsWith('/') ? path : `/${path}`}`;
}

/** A page of a language: page('pl', 'plugins') -> '/deckpilot-plugins/pl/plugins/'. */
export function page(lang: Lang, path = ''): string {
  const clean = path.replace(/^\/+|\/+$/g, '');
  const [route = '', hash] = clean.split('#');
  const [pathname = '', query] = route.split('?');
  const tail = pathname ? `${pathname.replace(/\/+$/, '')}/` : '';
  return `${BASE}/${lang}/${tail}${query ? `?${query}` : ''}${hash ? `#${hash}` : ''}`;
}

/**
 * The part of a pathname after the language: '/deckpilot-plugins/pl/docs/x/'
 * -> 'docs/x'. Used to link the same page in the other language.
 */
export function routeOf(pathname: string): string {
  const rest = pathname.startsWith(BASE) ? pathname.slice(BASE.length) : pathname;
  return rest.replace(/^\/(en|pl)(\/|$)/, '').replace(/\/+$/, '');
}

export function absolute(site: URL | undefined, path: string): string {
  return new URL(path, site ?? 'https://knittingcode.github.io').href;
}
