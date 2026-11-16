// Checks that English and Polish are complete mirrors of each other in a
// built site (stdlib only):
//
//   node tests/check-i18n.mjs <dist>
//
// - the same routes exist under /en/ and /pl/;
// - every page declares its own language, and links its counterpart with
//   hreflang (en, pl, x-default) and a self canonical;
// - a page links only to pages of its own language (the language switcher
//   excepted), so a visitor never falls into the other language;
// - the documentation has the same pages in both languages.
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';

const dist = resolve(process.argv[2] ?? 'dist');
const BASE = process.env.SITE_BASE ?? '/deckpilot';
const ORIGIN = 'https://knittingcode.github.io';
const LANGS = ['en', 'pl'];

function routes(lang) {
  const root = join(dist, lang);
  const walk = (dir) =>
    readdirSync(dir).flatMap((name) => {
      const path = join(dir, name);
      return statSync(path).isDirectory() ? walk(path) : [path.slice(root.length)];
    });
  return existsSync(root) ? walk(root).sort() : [];
}

const problems = [];
const [en, pl] = LANGS.map(routes);
for (const route of en) if (!pl.includes(route)) problems.push(`/pl${route} is missing (exists in English)`);
for (const route of pl) if (!en.includes(route)) problems.push(`/en${route} is missing (exists in Polish)`);

for (const lang of LANGS) {
  const other = lang === 'en' ? 'pl' : 'en';
  for (const route of routes(lang).filter((r) => r.endsWith('.html'))) {
    const html = readFileSync(join(dist, lang, route), 'utf8');
    const where = `/${lang}${route}`;
    const path = route.replace(/index\.html$/, '');
    if (!new RegExp(`<html lang="${lang}"`).test(html)) problems.push(`${where}: <html lang> is not "${lang}"`);
    const expect = (pattern, what) => {
      if (!html.includes(pattern)) problems.push(`${where}: missing ${what}`);
    };
    expect(`<link rel="canonical" href="${ORIGIN}${BASE}/${lang}${path}"`, 'self canonical');
    for (const code of LANGS) expect(`hreflang="${code}" href="${ORIGIN}${BASE}/${code}${path}"`, `hreflang ${code}`);
    expect(`hreflang="x-default" href="${ORIGIN}${BASE}/en${path}"`, 'hreflang x-default');
    const body = html.slice(html.indexOf('<body'));
    for (const match of body.matchAll(/<a\b([^>]*)>/g)) {
      const attrs = match[1];
      const href = /\shref="([^"]*)"/.exec(attrs)?.[1] ?? '';
      if (href.startsWith(`${BASE}/${other}/`) && !attrs.includes('data-set-lang')) {
        problems.push(`${where}: links to the other language (${href})`);
      }
    }
  }
}

for (const problem of problems) console.error(`error: ${problem}`);
console.log(`i18n: ${en.length} English and ${pl.length} Polish file(s) compared`);
process.exit(problems.length ? 1 : 0);
