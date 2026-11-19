import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { classify, compareTags, megabytes, published, select, signingFrom, type ApiRelease } from '../../src/lib/release-model.ts';

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

// The repository's releases as GitHub listed them after v0.8.0 (abridged:
// no assets), with an unpublished draft and pre-releases around it.
const live: ApiRelease[] = [
  ['v0.6.0-alpha.2', true, true, null],
  ['v0.8.0', false, false, '2026-11-19T01:38:25Z'],
  ['v0.7.0', false, false, '2026-11-18T02:35:01Z'],
  ['v0.6.0', false, false, '2026-11-17T15:07:47Z'],
  ['v0.6.0-alpha.4', false, true, '2026-11-16T21:56:37Z'],
  ['v0.6.0-alpha.3', false, true, '2026-11-14T20:52:06Z'],
].map(([tag_name, draft, prerelease, published_at]) => ({
  tag_name: tag_name as string,
  draft: draft as boolean,
  prerelease: prerelease as boolean,
  published_at: published_at as string | null,
  html_url: `https://github.com/knittingcode/deckpilot/releases/tag/${tag_name}`,
  assets: [],
}));

test('the newest stable release is offered, never a pre-release beside it', () => {
  const { primary, newerPrerelease } = select(live);
  assert.equal(primary?.tag, 'v0.8.0');
  assert.equal(primary?.prerelease, false);
  assert.equal(newerPrerelease, null);
  // A pre-release published after the stable release, with a higher
  // version, is mentioned, never offered as the stable download.
  const rc = { ...live[1]!, tag_name: 'v0.9.0-rc.1', prerelease: true, published_at: '2026-11-20T00:00:00Z' };
  assert.equal(select([rc, ...live]).primary?.tag, 'v0.8.0');
  assert.equal(select([rc, ...live]).newerPrerelease?.tag, 'v0.9.0-rc.1');
  // A pre-release that says nothing of being one by its tag is still one.
  const flagged = { ...rc, tag_name: 'v0.9.0' };
  assert.equal(select([flagged, ...live]).primary?.tag, 'v0.8.0');
});

test('a fix of an older line does not replace the newest stable release', () => {
  const backport = { ...live[2]!, tag_name: 'v0.7.1', published_at: '2026-11-21T00:00:00Z' };
  const oldRc = { ...live[2]!, tag_name: 'v0.7.2-rc.1', prerelease: true, published_at: '2026-11-22T00:00:00Z' };
  const { primary, newerPrerelease } = select([oldRc, backport, ...live]);
  assert.equal(primary?.tag, 'v0.8.0');
  assert.equal(newerPrerelease, null);
});

test('versions compare by semantic version precedence', () => {
  const order = ['v0.6.0-alpha.3', 'v0.6.0-alpha.4', 'v0.6.0-alpha.10', 'v0.6.0-beta.1', 'v0.6.0-rc.1', 'v0.6.0', 'v0.7.0', 'v0.10.0', 'v1.0.0'];
  assert.deepEqual([...order].reverse().sort(compareTags), order);
  assert.ok(compareTags('not-a-version', 'v0.0.1') < 0);
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
