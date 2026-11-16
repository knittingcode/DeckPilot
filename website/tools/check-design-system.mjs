// The shared design system (src/styles/shared) and the brand files are
// copies. This script keeps them honest (stdlib only).
//
//   node tools/check-design-system.mjs             offline, part of CI
//   node tools/check-design-system.mjs --upstream  also compare with the Marketplace repository
//   node tools/check-design-system.mjs --update    take the Marketplace repository's current files
//
// Offline it fails when
// - a shared stylesheet differs from the checksum recorded in
//   src/styles/shared/SOURCE.json (someone edited the copy instead of the
//   source), or
// - a brand file differs from the original in .github/assets/branding.
//
// --upstream downloads the three stylesheets from the main branch of
// knittingcode/deckpilot-plugins and reports the files that have changed
// there since the recorded commit. Drift is reported, with exit status 3,
// and never breaks a build or a deployment: the workflow runs it as a
// separate job that only warns. --update rewrites the copies and SOURCE.json.
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const shared = join(root, 'src/styles/shared');
const sourceFile = join(shared, 'SOURCE.json');
const source = JSON.parse(readFileSync(sourceFile, 'utf8'));
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
const args = new Set(process.argv.slice(2));
const problems = [];

for (const [name, expected] of Object.entries(source.files)) {
  const actual = sha256(readFileSync(join(shared, name)));
  if (actual !== expected) {
    problems.push(`src/styles/shared/${name} was edited here (sha256 ${actual}, recorded ${expected}). Change it in ${source.repository} and take it with --update.`);
  }
}

const branding = join(root, '../.github/assets/branding');
if (existsSync(branding)) {
  for (const name of ['icon.svg', 'icon.png', 'wordmark.svg', 'wordmark-dark.svg']) {
    const original = sha256(readFileSync(join(branding, name)));
    const copy = sha256(readFileSync(join(root, 'src/assets/brand', name)));
    if (original !== copy) problems.push(`src/assets/brand/${name} differs from .github/assets/branding/${name}; copy the original again.`);
  }
}

for (const problem of problems) console.error(`error: ${problem}`);
if (problems.length) process.exit(1);
console.log(`design system: ${Object.keys(source.files).length} shared stylesheet(s) match ${source.repository}@${source.commit.slice(0, 7)}; brand files match the originals`);

if (args.has('--upstream') || args.has('--update')) {
  const api = `https://api.github.com/repos/${source.repository}`;
  const headers = { 'User-Agent': 'design-system-check', Accept: 'application/vnd.github+json' };
  const head = await (await fetch(`${api}/commits/${source.branch}`, { headers, signal: AbortSignal.timeout(20000) })).json();
  if (typeof head.sha !== 'string') {
    console.error(`error: cannot read ${source.repository}@${source.branch}`);
    process.exit(2);
  }
  const drift = [];
  const fresh = {};
  for (const name of Object.keys(source.files)) {
    const url = `https://raw.githubusercontent.com/${source.repository}/${head.sha}/${source.path}/${name}`;
    const response = await fetch(url, { signal: AbortSignal.timeout(20000) });
    if (!response.ok) {
      console.error(`error: ${url} answered ${response.status}`);
      process.exit(2);
    }
    const bytes = Buffer.from(await response.arrayBuffer());
    fresh[name] = bytes;
    if (sha256(bytes) !== source.files[name]) drift.push(name);
  }
  if (drift.length === 0) {
    console.log(`design system: up to date with ${source.repository}@${head.sha.slice(0, 7)}`);
  } else if (args.has('--update')) {
    for (const name of Object.keys(fresh)) {
      writeFileSync(join(shared, name), fresh[name]);
      source.files[name] = sha256(fresh[name]);
    }
    source.commit = head.sha;
    writeFileSync(sourceFile, `${JSON.stringify(source, null, 2)}\n`);
    console.log(`design system: updated ${drift.join(', ')} to ${source.repository}@${head.sha.slice(0, 7)}; review the site in both themes`);
  } else {
    console.log(
      `design system drift: ${drift.join(', ')} changed in ${source.repository} since ${source.commit.slice(0, 7)} (now ${head.sha.slice(0, 7)}). Run "npm run check:design -- --update" in website/ and review the site.`,
    );
    process.exit(3);
  }
}
