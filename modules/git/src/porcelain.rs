//! Parsing `git status --porcelain=v2 --branch -z`, a format Git keeps
//! stable for scripts.

/// The branch part of a status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Branch {
    /// The checked-out commit; `None` before the first commit.
    pub head: Option<String>,
    /// The branch name; `None` when HEAD is detached.
    pub name: Option<String>,
    /// e.g. `origin/main`; `None` without a tracking branch.
    pub upstream: Option<String>,
    /// Commits ahead of and behind the upstream, as last fetched. `None`
    /// when there is no upstream or it is gone.
    pub ahead_behind: Option<(u32, u32)>,
}

impl Branch {
    pub fn detached(&self) -> bool {
        self.name.is_none() && self.head.is_some()
    }

    pub fn unborn(&self) -> bool {
        self.head.is_none()
    }
}

/// One changed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Two-letter status as `git status --short` shows it, e.g. `M.`, `??`.
    pub code: String,
    pub path: String,
    /// For renames and copies: where it came from.
    pub from: Option<String>,
}

impl Change {
    fn staged(&self) -> bool {
        let x = self.code.as_bytes()[0];
        x != b'.' && x != b'?' && !self.conflict()
    }

    fn unstaged(&self) -> bool {
        let y = self.code.as_bytes()[1];
        y != b'.' && y != b'?' && !self.conflict()
    }

    fn untracked(&self) -> bool {
        self.code == "??"
    }

    fn conflict(&self) -> bool {
        self.code.starts_with('U')
            || self.code.ends_with('U')
            || self.code == "AA"
            || self.code == "DD"
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: Branch,
    pub changes: Vec<Change>,
}

/// Counts by kind; a path changed both in the index and the work tree
/// counts in both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub conflicts: usize,
}

impl Status {
    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for change in &self.changes {
            counts.staged += usize::from(change.staged());
            counts.unstaged += usize::from(change.unstaged());
            counts.untracked += usize::from(change.untracked());
            counts.conflicts += usize::from(change.conflict());
        }
        counts
    }

    pub fn clean(&self) -> bool {
        self.changes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unexpected output from git status: {0}")]
pub struct ParseError(String);

/// Parses NUL-separated porcelain v2 output.
pub fn parse(output: &str) -> Result<Status, ParseError> {
    let mut status = Status::default();
    let mut records = output.split('\0').filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        let bad = || ParseError(record.chars().take(80).collect());
        if let Some(header) = record.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" => {
                    status.branch.head = (value != "(initial)").then(|| value.to_owned());
                }
                "branch.head" => {
                    status.branch.name = (value != "(detached)").then(|| value.to_owned());
                }
                "branch.upstream" => status.branch.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    let mut parts = value.split(' ');
                    let ahead = parts.next().and_then(|a| a.strip_prefix('+'));
                    let behind = parts.next().and_then(|b| b.strip_prefix('-'));
                    if let (Some(ahead), Some(behind)) = (ahead, behind) {
                        status.branch.ahead_behind = Some((
                            ahead.parse().map_err(|_| bad())?,
                            behind.parse().map_err(|_| bad())?,
                        ));
                    }
                }
                // `branch.oid`, stash counts and future headers.
                _ => {}
            }
            continue;
        }
        let (kind, rest) = record.split_once(' ').ok_or_else(bad)?;
        let change = match kind {
            // `1 XY sub mH mI mW hH hI path`
            "1" => {
                let fields: Vec<&str> = rest.splitn(8, ' ').collect();
                let [code, .., path] = fields.as_slice() else {
                    return Err(bad());
                };
                if fields.len() != 8 {
                    return Err(bad());
                }
                Change {
                    code: (*code).to_owned(),
                    path: (*path).to_owned(),
                    from: None,
                }
            }
            // `2 XY sub mH mI mW hH hI Xscore path` NUL `origPath`
            "2" => {
                let fields: Vec<&str> = rest.splitn(9, ' ').collect();
                if fields.len() != 9 {
                    return Err(bad());
                }
                Change {
                    code: fields[0].to_owned(),
                    path: fields[8].to_owned(),
                    from: Some(records.next().ok_or_else(bad)?.to_owned()),
                }
            }
            // `u XY sub m1 m2 m3 mW h1 h2 h3 path`
            "u" => {
                let fields: Vec<&str> = rest.splitn(10, ' ').collect();
                if fields.len() != 10 {
                    return Err(bad());
                }
                Change {
                    code: fields[0].to_owned(),
                    path: fields[9].to_owned(),
                    from: None,
                }
            }
            "?" => Change {
                code: "??".into(),
                path: rest.to_owned(),
                from: None,
            },
            // Ignored files are only listed when asked for.
            "!" => continue,
            _ => return Err(bad()),
        };
        if change.code.len() != 2 {
            return Err(bad());
        }
        status.changes.push(change);
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(lines: &[&str]) -> String {
        let mut out = lines.join("\0");
        out.push('\0');
        out
    }

    #[test]
    fn parses_branch_headers_and_changes() {
        let output = z(&[
            "# branch.oid 1234567890abcdef1234567890abcdef12345678",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "1 M. N... 100644 100644 100644 aaaa bbbb src/lib.rs",
            "1 .M N... 100644 100644 100644 aaaa bbbb README with spaces.md",
            "1 MM N... 100644 100644 100644 aaaa bbbb both.txt",
            "2 R. N... 100644 100644 100644 aaaa bbbb R100 new name.txt",
            "old name.txt",
            "u UU N... 100644 100644 100644 100644 a b c conflicted.rs",
            "? notes.txt",
            "! target",
        ]);
        let status = parse(&output).unwrap();
        assert_eq!(status.branch.name.as_deref(), Some("main"));
        assert_eq!(status.branch.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.branch.ahead_behind, Some((2, 1)));
        assert!(!status.branch.detached() && !status.branch.unborn());
        let paths: Vec<_> = status.changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "src/lib.rs",
                "README with spaces.md",
                "both.txt",
                "new name.txt",
                "conflicted.rs",
                "notes.txt"
            ]
        );
        assert_eq!(status.changes[3].from.as_deref(), Some("old name.txt"));
        assert_eq!(
            status.counts(),
            Counts {
                staged: 3,
                unstaged: 2,
                untracked: 1,
                conflicts: 1
            }
        );
        assert!(!status.clean());
    }

    #[test]
    fn detached_and_unborn_heads() {
        let detached = parse(&z(&[
            "# branch.oid 1234567890abcdef1234567890abcdef12345678",
            "# branch.head (detached)",
        ]))
        .unwrap();
        assert!(detached.branch.detached() && detached.branch.name.is_none());
        let unborn = parse(&z(&["# branch.oid (initial)", "# branch.head main"])).unwrap();
        assert!(unborn.branch.unborn() && !unborn.branch.detached());
        assert_eq!(unborn.branch.name.as_deref(), Some("main"));
        assert!(unborn.clean());
        // An upstream that is gone has no ahead/behind line.
        let gone = parse(&z(&[
            "# branch.oid 1234567890abcdef1234567890abcdef12345678",
            "# branch.head main",
            "# branch.upstream origin/main",
        ]))
        .unwrap();
        assert_eq!(gone.branch.ahead_behind, None);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("not porcelain\0").is_err());
        assert!(parse(&z(&["1 M. short"])).is_err());
        assert!(parse(&z(&["2 R. N... 1 2 3 a b R100 only-new"])).is_err());
    }
}
