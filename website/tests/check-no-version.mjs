// No page may name a DeckPilot version by hand: the release shown on the
// site is read from GitHub when it is built. This fails when a version
// number is written into the site's sources (stdlib only).
//
//   node tests/check-no-version.mjs
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const VERSION = /\bv?\d+\.\d+\.\d+-(alpha|beta|rc)\.\d+\b|\bv\d+\.\d+\.\d+\b/g;
const TEXT = /\.(astro|ts|mjs|md|css|json)$/;

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}

const problems = [];
for (const file of walk(join(root, 'src')).filter((path) => TEXT.test(path))) {
  const lines = readFileSync(file, 'utf8').split('\n');
  lines.forEach((line, i) => {
    for (const match of line.matchAll(VERSION)) problems.push(`${relative(root, file)}:${i + 1}: names the version ${match[0]}`);
  });
}

for (const problem of problems) console.error(`error: ${problem}`);
console.log(`versions: no DeckPilot version is written into src/${problems.length ? ' (failed)' : ''}`);
process.exit(problems.length ? 1 : 0);
