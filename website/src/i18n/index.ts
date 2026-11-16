import { en } from './en';
import { pl } from './pl';

export const LANGS = ['en', 'pl'] as const;
export type Lang = (typeof LANGS)[number];
export const DEFAULT_LANG: Lang = 'en';

/** The shape every language must fill completely (checked by the compiler). */
export type Dictionary = typeof en;

const dictionaries: Record<Lang, Dictionary> = { en, pl };

export function t(lang: Lang): Dictionary {
  return dictionaries[lang];
}

export function isLang(value: string | undefined): value is Lang {
  return (LANGS as readonly string[]).includes(value ?? '');
}

export const LANG_NAMES: Record<Lang, string> = { en: 'English', pl: 'Polski' };
export const OG_LOCALE: Record<Lang, string> = { en: 'en_US', pl: 'pl_PL' };
export const DATE_LOCALE: Record<Lang, string> = { en: 'en-GB', pl: 'pl-PL' };

/** getStaticPaths() for pages under /[lang]/. */
export function langPaths() {
  return LANGS.map((lang) => ({ params: { lang } }));
}

/** Replaces {name} placeholders. */
export function fill(template: string, values: Record<string, string | number>): string {
  return template.replace(/\{(\w+)\}/g, (_, key: string) => String(values[key] ?? `{${key}}`));
}
