/**
 * Reads DeckPilot's published releases from GitHub while the site is built,
 * so no page ever names a version by hand.
 *
 *   RELEASES_FILE=<path>   read the API answer from a file instead (tests)
 *   GITHUB_TOKEN=<token>   optional; only raises the API's rate limit
 *
 * A failed request fails the build: the previous deployment stays online,
 * which is better than a page that silently says "no release".
 */
import { readFileSync } from 'node:fs';
import { REPO_SLUG } from '../data/site';
import { select, signingFrom, type ApiRelease, type Selection } from './release-model';

const API = `https://api.github.com/repos/${REPO_SLUG}/releases?per_page=30`;

async function fetchJson(url: string, headers: Record<string, string>): Promise<unknown> {
  let last: unknown;
  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      const response = await fetch(url, { headers, redirect: 'follow', signal: AbortSignal.timeout(20000) });
      if (!response.ok) throw new Error(`${url} answered ${response.status}`);
      return await response.json();
    } catch (error) {
      last = error;
      await new Promise((resolve) => setTimeout(resolve, attempt * 1500));
    }
  }
  throw last instanceof Error ? last : new Error(String(last));
}

async function load(): Promise<Selection> {
  const file = process.env.RELEASES_FILE;
  let list: unknown;
  if (file) {
    list = JSON.parse(readFileSync(file, 'utf8'));
  } else {
    const token = process.env.GITHUB_TOKEN;
    list = await fetchJson(API, {
      Accept: 'application/vnd.github+json',
      'X-GitHub-Api-Version': '2022-11-28',
      'User-Agent': 'deckpilot-website-build',
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    });
  }
  if (!Array.isArray(list)) throw new Error('the release list is not an array');
  const selection = select(list as ApiRelease[]);
  const primary = selection.primary;
  if (primary?.metaUrl && !file) {
    // The release's own record of how each build was signed. Optional: a
    // release without it gets no signing statement.
    try {
      primary.signing = signingFrom(await fetchJson(primary.metaUrl, { 'User-Agent': 'deckpilot-website-build' }), primary.tag);
    } catch (error) {
      console.warn(`release-meta.json of ${primary.tag} could not be read: ${String(error)}`);
    }
  } else if (primary && file) {
    const meta = process.env.RELEASE_META_FILE;
    if (meta) primary.signing = signingFrom(JSON.parse(readFileSync(meta, 'utf8')), primary.tag);
  }
  return selection;
}

let cached: Promise<Selection> | undefined;

/** The releases the site shows; requested once per build. */
export function getReleases(): Promise<Selection> {
  cached ??= load();
  return cached;
}
