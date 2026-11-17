/**
 * Turns the answer of GitHub's "list releases" API into what the download
 * page shows. Pure functions, no I/O: src/lib/releases.ts does the reading,
 * tests/unit/release-model.test.ts covers the rules.
 *
 * Rules:
 * - a draft is never shown, whatever else it says;
 * - only assets served from this repository's release downloads are linked;
 * - a button exists only for an installer that the release really has;
 * - nothing is assumed about signing: it is read from the release's own
 *   release-meta.json when that file is present.
 */

export const DOWNLOAD_PREFIX = 'https://github.com/knittingcode/deckpilot/releases/download/';

export type ApiAsset = {
  name: string;
  size: number;
  browser_download_url: string;
  digest?: string | null;
};

export type ApiRelease = {
  tag_name: string;
  name?: string | null;
  draft: boolean;
  prerelease: boolean;
  published_at: string | null;
  html_url: string;
  assets: ApiAsset[];
};

export type Os = 'macos' | 'windows' | 'linux';
export type Arch = 'arm64' | 'x64';
export type Format = 'dmg' | 'exe' | 'msi' | 'appimage' | 'deb' | 'rpm';

export type Download = {
  os: Os;
  arch: Arch;
  format: Format;
  name: string;
  url: string;
  size: number;
  /** Hex SHA-256 as GitHub reports it for the asset, when it does. */
  sha256: string | null;
};

/** How a platform's build was code-signed, from release-meta.json. */
export type Signing = { codesign: string; notarized: boolean | null };

export type Release = {
  tag: string;
  version: string;
  prerelease: boolean;
  publishedAt: string;
  url: string;
  downloads: Download[];
  /** The release's SHA256SUMS file, when it has one. */
  checksumsUrl: string | null;
  /** The release's release-meta.json, when it has one. */
  metaUrl: string | null;
  /** Whether the release carries signed update packages (*.sig). */
  updaterSignatures: boolean;
  /** Per OS and architecture, when release-meta.json was read. */
  signing: Partial<Record<`${Os}-${Arch}`, Signing>>;
};

export type Selection = {
  /** The release the page offers: the newest stable one, else the newest pre-release. */
  primary: Release | null;
  /** A pre-release newer than the stable release on offer. */
  newerPrerelease: Release | null;
};

const ARCH: Record<string, Arch> = { aarch64: 'arm64', arm64: 'arm64', x64: 'x64', amd64: 'x64', x86_64: 'x64' };
const FORMATS: [RegExp, Os, Format][] = [
  [/\.dmg$/i, 'macos', 'dmg'],
  [/-setup\.exe$/i, 'windows', 'exe'],
  [/\.msi$/i, 'windows', 'msi'],
  [/\.AppImage$/i, 'linux', 'appimage'],
  [/\.deb$/i, 'linux', 'deb'],
  [/\.rpm$/i, 'linux', 'rpm'],
];

function fromThisRepository(url: string): boolean {
  return url.toLowerCase().startsWith(DOWNLOAD_PREFIX);
}

/** An installer, or null for everything else (signatures, updater archives, manifests). */
export function classify(asset: ApiAsset): Download | null {
  if (!fromThisRepository(asset.browser_download_url)) return null;
  const kind = FORMATS.find(([pattern]) => pattern.test(asset.name));
  if (!kind) return null;
  const token = /(?:^|[_.-])(aarch64|arm64|x86_64|amd64|x64)(?=[_.-])/i.exec(asset.name)?.[1]?.toLowerCase();
  const arch = token ? ARCH[token] : undefined;
  if (!arch) return null;
  const digest = /^sha256:([0-9a-f]{64})$/i.exec(asset.digest ?? '')?.[1]?.toLowerCase() ?? null;
  return { os: kind[1], arch, format: kind[2], name: asset.name, url: asset.browser_download_url, size: asset.size, sha256: digest };
}

const OS_ORDER: Os[] = ['macos', 'windows', 'linux'];
const FORMAT_ORDER: Format[] = ['dmg', 'exe', 'msi', 'appimage', 'deb', 'rpm'];

function toRelease(api: ApiRelease): Release {
  const downloads = api.assets
    .map(classify)
    .filter((download): download is Download => download !== null)
    .sort(
      (a, b) =>
        OS_ORDER.indexOf(a.os) - OS_ORDER.indexOf(b.os) ||
        (a.arch === b.arch ? 0 : a.arch === 'arm64' ? -1 : 1) ||
        FORMAT_ORDER.indexOf(a.format) - FORMAT_ORDER.indexOf(b.format),
    );
  const named = (name: string) => api.assets.find((asset) => asset.name === name && fromThisRepository(asset.browser_download_url));
  return {
    tag: api.tag_name,
    version: api.tag_name.replace(/^v/, ''),
    prerelease: api.prerelease,
    publishedAt: api.published_at ?? '',
    url: api.html_url,
    downloads,
    checksumsUrl: named('SHA256SUMS')?.browser_download_url ?? null,
    metaUrl: named('release-meta.json')?.browser_download_url ?? null,
    updaterSignatures: api.assets.some((asset) => asset.name.endsWith('.sig')),
    signing: {},
  };
}

/** Published releases only, newest first. A draft has no place on the site. */
export function published(list: ApiRelease[]): ApiRelease[] {
  return list
    .filter((release) => release.draft === false && typeof release.published_at === 'string' && release.published_at !== '')
    .sort((a, b) => Date.parse(b.published_at ?? '') - Date.parse(a.published_at ?? ''));
}

export function select(list: ApiRelease[]): Selection {
  const releases = published(list);
  const newest = releases[0];
  if (!newest) return { primary: null, newerPrerelease: null };
  const stable = releases.find((release) => !release.prerelease);
  if (!stable) return { primary: toRelease(newest), newerPrerelease: null };
  return { primary: toRelease(stable), newerPrerelease: newest.prerelease ? toRelease(newest) : null };
}

const TARGETS: Record<string, `${Os}-${Arch}`> = {
  'aarch64-apple-darwin': 'macos-arm64',
  'x86_64-apple-darwin': 'macos-x64',
  'x86_64-pc-windows-msvc': 'windows-x64',
  'aarch64-pc-windows-msvc': 'windows-arm64',
  'x86_64-unknown-linux-gnu': 'linux-x64',
  'aarch64-unknown-linux-gnu': 'linux-arm64',
};

/**
 * Reads the signing facts out of a release's release-meta.json. Anything
 * that is not exactly the expected shape, or that describes another tag, is
 * ignored: the page then makes no signing statement for that platform.
 */
export function signingFrom(meta: unknown, tag: string): Release['signing'] {
  const result: Release['signing'] = {};
  if (typeof meta !== 'object' || meta === null) return result;
  const record = meta as { tag?: unknown; platforms?: unknown };
  if (record.tag !== tag || typeof record.platforms !== 'object' || record.platforms === null) return result;
  for (const [target, value] of Object.entries(record.platforms as Record<string, unknown>)) {
    const key = TARGETS[target];
    if (!key || typeof value !== 'object' || value === null) continue;
    const { codesign, notarized } = value as { codesign?: unknown; notarized?: unknown };
    if (typeof codesign !== 'string') continue;
    result[key] = { codesign, notarized: typeof notarized === 'boolean' ? notarized : null };
  }
  return result;
}

/** 15455368 -> '14.7 MB' (binary megabytes, one decimal). */
export function megabytes(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
