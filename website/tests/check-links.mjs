// Checks every link of a built site (stdlib only).
//
//   node tests/check-links.mjs <dist> [--external]
//
// Internal: every href/src that points into the site must resolve to a file
// of the build, and every #fragment to an id on the target page. Links must
// stay under the base path; the Plugin Marketplace, a separate site on the
// same host, counts as external.
// --external: also requests every external URL once (HEAD, then GET) and
// fails on a 4xx/5xx answer. Not part of CI by default: it depends on the
// network and on other sites being up.
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';

const args = process.argv.slice(2);
const dist = resolve(args.find((a) => !a.startsWith('--')) ?? 'dist');
const external = args.includes('--external');
const BASE = process.env.SITE_BASE ?? '/deckpilot';
const ORIGIN = 'https://knittingcode.github.io';
const SISTER = '/deckpilot-plugins';

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}

const pages = walk(dist).filter((file) => file.endsWith('.html'));
const ids = new Map();
const idsOf = (file) => {
  if (!ids.has(file)) ids.set(file, new Set([...readFileSync(file, 'utf8').matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));
  return ids.get(file);
};
// One pass, so an already decoded ampersand is never decoded again.
const decode = (text) => text.replace(/&(?:amp|#38|#x26);/g, '&');
// Schemes that are not addresses to check, and schemes that run code.
const SKIPPED = new Set(['mailto:', 'tel:', 'data:']);
const SCRIPT = new Set(['javascript:', 'vbscript:']);

const problems = [];
const externals = new Map();
let checked = 0;

for (const file of pages) {
  const html = readFileSync(file, 'utf8');
  const rel = file.slice(dist.length);
  const pageUrl = new URL(`${BASE}${rel.replace(/index\.html$/, '')}`, ORIGIN);
  const refs = [...html.matchAll(/<(?:a|link|img|script|source)\b[^>]*?\s(?:href|src)="([^"]*)"/g)].map((m) => decode(m[1]));
  for (const ref of refs) {
    const url = new URL(ref.trim(), pageUrl);
    if (SCRIPT.has(url.protocol)) {
      problems.push(`${rel}: ${url.protocol} link`);
      continue;
    }
    if (SKIPPED.has(url.protocol)) continue;
    // The Plugin Marketplace is a separate site on the same host: external.
    const sister = url.origin === ORIGIN && (url.pathname === SISTER || url.pathname.startsWith(`${SISTER}/`));
    if (url.origin !== ORIGIN || sister) {
      if (url.protocol !== 'https:') problems.push(`${rel}: ${ref} is not https`);
      const list = externals.get(url.href.split('#')[0]) ?? [];
      list.push(rel);
      externals.set(url.href.split('#')[0], list);
      continue;
    }
    checked++;
    if (url.pathname !== BASE && !url.pathname.startsWith(`${BASE}/`)) {
      problems.push(`${rel}: ${ref} leaves the base path ${BASE}/`);
      continue;
    }
    const path = decodeURIComponent(url.pathname.slice(BASE.length));
    let target = join(dist, path);
    if (path.endsWith('/')) target = join(target, 'index.html');
    else if (existsSync(target) && statSync(target).isDirectory()) {
      problems.push(`${rel}: ${ref} needs a trailing slash`);
      continue;
    }
    if (!existsSync(target)) {
      problems.push(`${rel}: ${ref} does not exist in the build`);
      continue;
    }
    if (url.hash.length > 1 && target.endsWith('.html')) {
      const id = decodeURIComponent(url.hash.slice(1));
      if (!idsOf(target).has(id)) problems.push(`${rel}: ${ref} has no element with id "${id}"`);
    }
  }
}

if (external) {
  const urls = [...externals.keys()].sort();
  const queue = [...urls];
  const fetchOne = async (url) => {
    for (const method of ['HEAD', 'GET']) {
      try {
        const response = await fetch(url, { method, redirect: 'follow', headers: { 'User-Agent': 'link-check' }, signal: AbortSignal.timeout(20000) });
        if (response.status < 400) return;
        if (method === 'GET') problems.push(`external ${response.status}: ${url} (from ${externals.get(url)[0]})`);
      } catch (error) {
        if (method === 'GET') problems.push(`external ${error.name}: ${url} (from ${externals.get(url)[0]})`);
      }
    }
  };
  await Promise.all(Array.from({ length: 6 }, async () => {
    while (queue.length) await fetchOne(queue.shift());
  }));
  console.log(`links: ${urls.length} external URL(s) requested`);
}

for (const problem of problems) console.error(`error: ${problem}`);
console.log(`links: ${pages.length} page(s), ${checked} internal link(s), ${externals.size} distinct external URL(s)`);
process.exit(problems.length ? 1 : 0);
