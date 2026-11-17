// Builds the site twice more from invented release lists, so the tests can
// look at states the real repository is not in today. Never deployed: the
// workflow uploads dist/ only, and every fixture build is marked.
//
//   dist-fixtures/none     no published release at all
//   dist-fixtures/stable   a stable release, a newer pre-release, a draft,
//                          and an asset hosted somewhere else
import { spawnSync } from 'node:child_process';
import { rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const fixtures = join(root, 'tests/fixtures');
const builds = [
  { name: 'none', env: { RELEASES_FILE: join(fixtures, 'releases-none.json') } },
  {
    name: 'stable',
    env: { RELEASES_FILE: join(fixtures, 'releases-stable.json'), RELEASE_META_FILE: join(fixtures, 'release-meta-stable.json') },
  },
];

rmSync(join(root, 'dist-fixtures'), { recursive: true, force: true });
for (const build of builds) {
  const out = join(root, 'dist-fixtures', build.name);
  const result = spawnSync('npx', ['astro', 'build', '--silent'], {
    cwd: root,
    stdio: 'inherit',
    env: { ...process.env, ...build.env, SITE_OUT_DIR: out },
  });
  if (result.status !== 0) process.exit(result.status ?? 1);
  writeFileSync(join(out, 'FIXTURE_BUILD'), 'test data, never deploy\n');
  console.log(`built dist-fixtures/${build.name}`);
}
