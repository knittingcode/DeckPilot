//! `prepare-release <version>`: the release-preparation pull request.
//!
//! On a new branch from `origin/main` it sets the version everywhere
//! (`set-version`), moves the changelog's "Unreleased" notes into the
//! version's section, validates the result as the release workflow's tag
//! gate will (`check-versions --tag`), commits, pushes and opens a pull
//! request. It never creates, moves or pushes a tag: after the pull request
//! is merged and CI has passed on the merge commit, a maintainer tags that
//! commit by hand (`docs/releasing.md`).

use std::path::Path;
use std::process::Command;

use deckpilot_update::channel::{ReleaseKind, ReleaseTag};
use semver::Version;

const CHANGELOG: &str = "CHANGELOG.md";
/// Files `set-version` and the changelog step change; nothing else is
/// committed.
const CHANGED: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "apps/desktop/package.json",
    "apps/desktop/src-tauri/tauri.conf.json",
    CHANGELOG,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publish {
    /// Push the branch and open the pull request (default).
    PullRequest,
    /// Commit on the local branch only (`--local`).
    Local,
}

/// The heading of a `## [x]` section, if `line` is one.
fn section_version(line: &str) -> Option<&str> {
    line.strip_prefix("## [")?.split(']').next()
}

/// Moves the notes of `## [Unreleased]` into the section of `version`'s
/// release (`X.Y.Z`, also for its pre-releases, whose notes come from it)
/// and returns the new changelog. A stable release dates its section
/// (`## [X.Y.Z] - <date>`); a pre-release keeps `- Unreleased`. An empty
/// `## [Unreleased]` heading stays for the next changes. Fails if the
/// version's notes end up empty.
pub fn prepare_changelog(text: &str, version: &Version, date: &str) -> Result<String, String> {
    let base = format!("{}.{}.{}", version.major, version.minor, version.patch);
    let stable = version.pre.is_empty();
    let heading = if stable {
        format!("## [{base}] - {date}")
    } else {
        format!("## [{base}] - Unreleased")
    };
    let lines: Vec<&str> = text.lines().collect();
    let sections: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| section_version(line).is_some())
        .map(|(i, _)| i)
        .collect();
    let end_of = |start: usize| {
        sections
            .iter()
            .copied()
            .find(|&i| i > start)
            .unwrap_or(lines.len())
    };
    let find = |name: &str| {
        sections
            .iter()
            .copied()
            .find(|&i| section_version(lines[i]).is_some_and(|v| v.eq_ignore_ascii_case(name)))
    };

    let unreleased = find("Unreleased");
    let moved: Vec<&str> =
        unreleased.map_or_else(Vec::new, |start| lines[start + 1..end_of(start)].to_vec());
    let moved = moved.join("\n").trim().to_owned();
    let existing = find(&base);

    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if Some(i) == unreleased {
            out.push(lines[i].to_owned());
            out.push(String::new());
            if existing.is_none() {
                out.push(heading.clone());
                out.push(String::new());
                out.push(moved.clone());
                out.push(String::new());
            }
            i = end_of(i);
            continue;
        }
        if Some(i) == existing {
            out.push(heading.clone());
            out.push(String::new());
            if !moved.is_empty() {
                out.push(moved.clone());
                out.push(String::new());
            }
            let body = lines[i + 1..end_of(i)].join("\n");
            let body = body.trim();
            if !body.is_empty() {
                out.push(body.to_owned());
                out.push(String::new());
            }
            i = end_of(i);
            continue;
        }
        if existing.is_none() && unreleased.is_none() && sections.first() == Some(&i) {
            // No section at all for it: an empty one, refused below.
            out.push(heading.clone());
            out.push(String::new());
        }
        out.push(lines[i].to_owned());
        i += 1;
    }
    let mut result = out.join("\n");
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    let result = format!("{}\n", result.trim_end());
    crate::release::release_notes(&result, &version.to_string())
        .map_err(|e| format!("{e}: add the notes under \"## [Unreleased]\" first"))?;
    Ok(result)
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The versions of all `v*` tags, locally and on `origin`.
fn tagged_versions(root: &Path) -> Result<Vec<(String, Option<Version>)>, String> {
    let mut tags: Vec<String> = git(root, &["tag", "--list", "v*"])?
        .lines()
        .map(str::to_owned)
        .collect();
    let remote = git(root, &["ls-remote", "--tags", "--refs", "origin", "v*"])?;
    tags.extend(
        remote
            .lines()
            .filter_map(|line| line.split("refs/tags/").nth(1))
            .map(str::to_owned),
    );
    tags.sort();
    tags.dedup();
    Ok(tags
        .into_iter()
        .map(|tag| {
            let version = Version::parse(tag.trim_start_matches('v')).ok();
            (tag, version)
        })
        .collect())
}

/// Today in UTC, `YYYY-MM-DD`.
fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Refuses a dirty checkout, a tag that exists or is not newer than every
/// existing `v*` tag (locally or on `origin`), and an existing branch.
/// Returns the branch to create.
fn preflight(root: &Path, tag: &ReleaseTag) -> Result<String, String> {
    let name = &tag.tag;
    if !git(root, &["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
        return Err("the checkout has uncommitted changes; commit or stash them first".into());
    }
    git(root, &["fetch", "--quiet", "--tags", "origin", "main"])?;
    for (existing, existing_version) in tagged_versions(root)? {
        if existing == *name {
            return Err(format!(
                "{name} already exists; a tag is never reused, choose the next version"
            ));
        }
        if let Some(existing_version) = existing_version
            && existing_version >= tag.version
        {
            return Err(format!(
                "{name} is not newer than the existing tag {existing}; choose a higher version"
            ));
        }
    }
    let branch = format!("release/prepare-{name}");
    let branch_ref = format!("refs/heads/{branch}");
    if git(root, &["rev-parse", "--verify", "--quiet", &branch_ref]).is_ok()
        || !git(root, &["ls-remote", "--heads", "origin", &branch])?.is_empty()
    {
        return Err(format!("the branch {branch} already exists"));
    }
    Ok(branch)
}

/// Opens the release-preparation pull request of the pushed `branch`.
fn open_pull_request(root: &Path, tag: &ReleaseTag, branch: &str) -> Result<(), String> {
    let (name, version) = (&tag.tag, &tag.version);
    let kind = match tag.kind {
        ReleaseKind::Stable => "stable release (stable channel, becomes latest if newest)",
        ReleaseKind::Prerelease => "pre-release (prerelease channel, never latest)",
    };
    let base = format!("{}.{}.{}", version.major, version.minor, version.patch);
    let body = format!(
        "Prepares **{name}**, a {kind}.\n\n\
         - version set to `{version}` in `Cargo.toml`, `Cargo.lock`, `apps/desktop/package.json` \
         and `apps/desktop/src-tauri/tauri.conf.json` (`deckpilot-dev set-version`);\n\
         - the notes under \"Unreleased\" moved into the `{base}` section of `CHANGELOG.md`;\n\
         - `deckpilot-dev check-versions --tag {name}` passes.\n\n\
         This pull request does not create a tag. After it is merged and the required checks have \
         passed on the merge commit, tag that commit (docs/releasing.md):\n\n\
         ```sh\ngit fetch origin\ngit tag -s {name} <merge-commit-sha> -m \"DeckPilot {version}\"\n\
         git push origin {name}\n```\n"
    );
    let title = format!("chore(release): prepare {name}");
    let output = Command::new(crate::program("gh"))
        .current_dir(root)
        .args(["pr", "create", "--base", "main", "--head", branch])
        .args(["--title", &title, "--body", &body])
        .output()
        .map_err(|e| format!("gh: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "the branch {branch} is pushed, but the pull request could not be created: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    println!("{}", String::from_utf8_lossy(&output.stdout).trim());
    Ok(())
}

/// Prepares the release of `version` in the checkout at `root`. Returns
/// the branch name.
pub fn prepare(root: &Path, version: &str, publish: Publish) -> Result<String, String> {
    let tag = ReleaseTag::for_version(version).map_err(|e| e.to_string())?;
    tag.msi_version()?;
    let name = tag.tag.clone();
    let branch = preflight(root, &tag)?;
    git(
        root,
        &[
            "switch",
            "--quiet",
            "-c",
            &branch,
            "refs/remotes/origin/main",
        ],
    )?;

    // The changelog first: set-version validates that the version has notes.
    let changelog_path = root.join(CHANGELOG);
    let changelog = std::fs::read_to_string(&changelog_path).map_err(|e| e.to_string())?;
    let prepared = prepare_changelog(&changelog, &tag.version, &today())?;
    std::fs::write(&changelog_path, prepared).map_err(|e| e.to_string())?;
    crate::versions::set(root, version)?;
    // As the release workflow's tag gate will check it.
    crate::versions::check(root, Some(&name))?;
    let changed = git(root, &["diff", "--name-only", "HEAD"])?;
    if let Some(file) = changed.lines().find(|file| !CHANGED.contains(file)) {
        return Err(format!(
            "unexpected change to {file}; nothing was committed"
        ));
    }
    let mut add = vec!["add", "--"];
    add.extend_from_slice(CHANGED);
    git(root, &add)?;
    let message = format!("chore(release): prepare {name}");
    git(root, &["commit", "--quiet", "-m", &message])?;
    if publish == Publish::PullRequest {
        git(root, &["push", "--quiet", "-u", "origin", &branch])?;
        open_pull_request(root, &tag, &branch)?;
    }
    Ok(branch)
}

/// `prepare-release <version> [--local]`.
pub fn command(root: &Path, args: &[String]) -> Result<(), String> {
    let version = args
        .first()
        .filter(|v| !v.starts_with("--"))
        .ok_or("usage: prepare-release <version> [--local]")?;
    let publish = if args.iter().any(|a| a == "--local") {
        Publish::Local
    } else {
        Publish::PullRequest
    };
    let branch = prepare(root, version, publish)?;
    println!(
        "prepared {version} on {branch}; no tag was created. After the pull request is merged \
         and CI passed on the merge commit, tag that commit (docs/releasing.md)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n### Fixed\n\n- New fix.\n\n\
                       ## [0.6.0] - Unreleased\n\n### Added\n\n- Feature.\n\n## [0.5.0] - Not released\n\n- Old.\n";

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn a_stable_release_dates_its_section_and_takes_the_unreleased_notes() {
        let out = prepare_changelog(LOG, &v("0.6.0"), "2026-11-12").unwrap();
        assert_eq!(
            out,
            "# Changelog\n\nIntro.\n\n## [Unreleased]\n\n## [0.6.0] - 2026-11-12\n\n### Fixed\n\n\
             - New fix.\n\n### Added\n\n- Feature.\n\n## [0.5.0] - Not released\n\n- Old.\n"
        );
        // Idempotent apart from the moved notes.
        assert_eq!(
            prepare_changelog(&out, &v("0.6.0"), "2026-11-12").unwrap(),
            out
        );
    }

    #[test]
    fn a_pre_release_collects_notes_in_the_undated_section() {
        let out = prepare_changelog(LOG, &v("0.6.0-alpha.1"), "2026-11-12").unwrap();
        assert!(out.contains("## [Unreleased]\n\n## [0.6.0] - Unreleased\n\n### Fixed"));
        assert!(!out.contains("2026-11-12"));
        assert!(crate::release::release_notes(&out, "0.6.0-alpha.1").is_ok());
    }

    #[test]
    fn a_new_version_gets_a_section_from_the_unreleased_notes() {
        let out = prepare_changelog(LOG, &v("0.7.0-beta.1"), "2026-11-12").unwrap();
        assert!(out.contains(
            "## [Unreleased]\n\n## [0.7.0] - Unreleased\n\n### Fixed\n\n- New fix.\n\n## [0.6.0] - Unreleased"
        ));
        // Without an Unreleased section, the version's existing notes count.
        let plain = "# Changelog\n\n## [0.6.0] - Unreleased\n\n- Feature.\n";
        assert_eq!(
            prepare_changelog(plain, &v("0.6.0"), "2026-11-12").unwrap(),
            "# Changelog\n\n## [0.6.0] - 2026-11-12\n\n- Feature.\n"
        );
    }

    #[test]
    fn a_release_without_notes_is_refused() {
        let empty = "# Changelog\n\n## [Unreleased]\n\n## [0.5.0]\n\n- Old.\n";
        assert!(
            prepare_changelog(empty, &v("0.6.0"), "2026-11-12")
                .unwrap_err()
                .contains("Unreleased")
        );
        assert!(prepare_changelog("# Changelog\n", &v("0.6.0"), "d").is_err());
    }

    #[test]
    fn the_repository_changelog_can_be_prepared() {
        let own =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../CHANGELOG.md"))
                .unwrap();
        let current = v(env!("CARGO_PKG_VERSION"));
        let next = Version::new(current.major, current.minor, current.patch);
        let stable = prepare_changelog(&own, &next, "2026-11-12").unwrap();
        // Without "Unreleased" notes only the version's heading changes.
        if !own.contains("## [Unreleased]") {
            let changed: Vec<_> = own
                .lines()
                .zip(stable.lines())
                .filter(|(a, b)| a != b)
                .collect();
            assert_eq!(changed.len(), 1, "{changed:?}");
            assert_eq!(changed[0].1, format!("## [{next}] - 2026-11-12"));
            let mut pre = next.clone();
            pre.pre = semver::Prerelease::new("alpha.1").unwrap();
            assert_eq!(prepare_changelog(&own, &pre, "2026-11-12").unwrap(), own);
        }
    }

    /// A checkout with the version files and an `origin` (a bare
    /// repository) with `main` and the given tags.
    fn checkout(tags: &[&str]) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = std::env::temp_dir().join(format!("deckpilot-prepare-{}", uuid::Uuid::new_v4()));
        let (work, origin) = (dir.join("work"), dir.join("origin.git"));
        std::fs::create_dir_all(&work).unwrap();
        for file in CHANGED {
            let target = work.join(file);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(root.join(file), target).unwrap();
        }
        let mut log = std::fs::read_to_string(work.join(CHANGELOG)).unwrap();
        let first = log.find("\n## [").unwrap();
        log.insert_str(first + 1, "## [Unreleased]\n\n- A change.\n\n");
        std::fs::write(work.join(CHANGELOG), log).unwrap();
        let run = |dir: &Path, args: &[&str]| git(dir, args).unwrap();
        run(&work, &["init", "--quiet", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Test"),
            ("user.email", "test@example.invalid"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
        ] {
            run(&work, &["config", key, value]);
        }
        run(&work, &["add", "-A"]);
        run(&work, &["commit", "--quiet", "-m", "init"]);
        for tag in tags {
            run(&work, &["tag", tag]);
        }
        run(&dir, &["clone", "--quiet", "--bare", "work", "origin.git"]);
        run(
            &work,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        // Tags on origin only, as after a fresh clone.
        for tag in tags {
            run(&work, &["tag", "-d", tag]);
        }
        (work, dir)
    }

    #[test]
    fn prepares_a_branch_and_never_tags() {
        let (work, dir) = checkout(&["v0.5.0-2"]);
        let branch = prepare(&work, "0.6.0-alpha.1", Publish::Local).unwrap();
        assert_eq!(branch, "release/prepare-v0.6.0-alpha.1");
        assert_eq!(
            git(&work, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap(),
            branch
        );
        assert_eq!(
            crate::versions::check(&work, Some("v0.6.0-alpha.1")).unwrap(),
            "0.6.0-alpha.1"
        );
        let changelog = std::fs::read_to_string(work.join(CHANGELOG)).unwrap();
        assert!(changelog.contains("## [Unreleased]\n\n## [0.6.0] - Unreleased\n\n- A change."));
        assert_eq!(
            git(&work, &["log", "-1", "--format=%s"]).unwrap(),
            "chore(release): prepare v0.6.0-alpha.1"
        );
        let changed = git(&work, &["diff", "--name-only", "HEAD~1"]).unwrap();
        assert!(changed.lines().all(|f| CHANGED.contains(&f)), "{changed}");
        // No tag anywhere but the one that existed.
        assert_eq!(git(&work, &["tag", "--list"]).unwrap(), "v0.5.0-2");
        assert_eq!(
            git(&dir.join("origin.git"), &["tag", "--list"]).unwrap(),
            "v0.5.0-2"
        );
        assert_eq!(
            git(&dir.join("origin.git"), &["branch", "--list", "release/*"]).unwrap(),
            ""
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_reused_older_or_malformed_versions_and_dirty_checkouts() {
        let (work, dir) = checkout(&["v0.6.0-alpha.1"]);
        let refused = |version: &str| prepare(&work, version, Publish::Local).unwrap_err();
        assert!(refused("0.6.0-alpha.1").contains("already exists"));
        assert!(refused("0.5.9").contains("not newer"));
        assert!(refused("0.6.0-1").contains("not a release tag"));
        assert!(refused("v0.6.0").contains("not a release tag"));
        assert!(refused("256.0.0").contains("MSI"));
        std::fs::write(work.join("Cargo.toml"), "changed").unwrap();
        assert!(refused("0.6.0-alpha.2").contains("uncommitted"));
        // Nothing was created.
        assert_eq!(
            git(&work, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap(),
            "main"
        );
        // Only the existing tag, fetched from origin; none created.
        assert_eq!(git(&work, &["tag", "--list"]).unwrap(), "v0.6.0-alpha.1");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
