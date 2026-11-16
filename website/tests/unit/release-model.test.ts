import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { classify, megabytes, published, select, signingFrom, type ApiRelease } from '../../src/lib/release-model.ts';

const read = (name: string) => JSON.parse(readFileSync(new URL(`../fixtures/${name}`, import.meta.url), 'utf8'));
const stable = read('releases-stable.json') as ApiRelease[];
const PREFIX = 'https://github.com/knittingcode/deckpilot/releases/download/v1.0.0/';
const asset = (name: string, url = `${PREFIX}${name}`) => ({ name, size: 1, browser_download_url: url, digest: null });

test('a draft is never published, whatever its other fields say', () => {
  assert.deepEqual(
    published(stable).map((release) => release.tag_name),
    ['v1.1.0-beta.1', 'v1.0.0'],
  );
  const onlyDraft = stable.filter((release) => release.draft);
  assert.equal(select(onlyDraft).primary, null);
  // A draft that claims a publication date is still a draft.
  assert.equal(select([{ ...onlyDraft[0]!, published_at: '2031-01-01T00:00:00Z' }]).primary, null);
});

test('no release gives an empty selection', () => {
  assert.deepEqual(select([]), { primary: null, newerPrerelease: null });
});

test('a stable release is offered before a newer pre-release, which is mentioned', () => {
  const { primary, newerPrerelease } = select(stable);
  assert.equal(primary?.tag, 'v1.0.0');
  assert.equal(primary?.prerelease, false);
  assert.equal(newerPrerelease?.tag, 'v1.1.0-beta.1');
});

test('without a stable release the newest pre-release is offered', () => {
  const list = stable.filter((release) => release.prerelease || release.draft);
  const { primary, newerPrerelease } = select(list);
  assert.equal(primary?.tag, 'v1.1.0-beta.1');
  assert.equal(primary?.prerelease, true);
  assert.equal(newerPrerelease, null);
});

test('only installers become downloads, and only from this repository', () => {
  const { primary } = select(stable);
  assert.deepEqual(
    primary?.downloads.map((download) => `${download.os}/${download.arch}/${download.format}`),
    ['macos/arm64/dmg', 'windows/x64/exe', 'linux/x64/rpm'],
  );
  assert.deepEqual([...new Set(primary?.downloads.map((download) => new URL(download.url).hostname))], ['github.com']);
  assert.equal(primary?.checksumsUrl, `${PREFIX}SHA256SUMS`);
  assert.equal(primary?.metaUrl, `${PREFIX}release-meta.json`);
  assert.equal(primary?.updaterSignatures, true);
  assert.equal(primary?.downloads[0]?.sha256, '1'.repeat(64));
  assert.equal(primary?.downloads[2]?.sha256, null);
});

test('file names of the published pre-release are recognised', () => {
  const kinds = [
    'DeckPilot_0.6.0-alpha.3_aarch64.dmg',
    'DeckPilot_0.6.0-alpha.3_x64.dmg',
    'DeckPilot_0.6.0-alpha.3_x64-setup.exe',
    'DeckPilot_0.6.0-alpha.3_x64_en-US.msi',
    'DeckPilot_0.6.0-alpha.3_amd64.AppImage',
    'DeckPilot_0.6.0-alpha.3_amd64.deb',
  ].map((name) => {
    const download = classify(asset(name));
    return download && `${download.os}/${download.arch}/${download.format}`;
  });
  assert.deepEqual(kinds, ['macos/arm64/dmg', 'macos/x64/dmg', 'windows/x64/exe', 'windows/x64/msi', 'linux/x64/appimage', 'linux/x64/deb']);
});

test('signatures, updater archives and manifests are not downloads', () => {
  for (const name of [
    'DeckPilot_0.6.0-alpha.3_amd64.deb.sig',
    'DeckPilot_aarch64.app.tar.gz',
    'DeckPilot_aarch64.app.tar.gz.sig',
    'prerelease.json',
    'latest.json',
    'release-meta.json',
    'SHA256SUMS',
  ]) {
    assert.equal(classify(asset(name)), null, name);
  }
  assert.equal(classify(asset('DeckPilot_1.0.0_x64.dmg', 'https://github.com/someone/else/releases/download/v1/DeckPilot_1.0.0_x64.dmg')), null);
});

test('signing facts come from release-meta.json of the same tag only', () => {
  const meta = read('release-meta-stable.json');
  assert.deepEqual(signingFrom(meta, 'v1.0.0'), {
    'macos-arm64': { codesign: 'developer-id', notarized: true },
    'windows-x64': { codesign: 'authenticode', notarized: null },
  });
  assert.deepEqual(signingFrom(meta, 'v1.0.1'), {});
  assert.deepEqual(signingFrom(null, 'v1.0.0'), {});
  assert.deepEqual(signingFrom({ tag: 'v1.0.0', platforms: { 'x86_64-pc-windows-msvc': { codesign: 7 } } }, 'v1.0.0'), {});
});

test('sizes are shown in megabytes', () => {
  assert.equal(megabytes(15455368), '14.7 MB');
});
