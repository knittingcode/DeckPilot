//! Live key state of `git.status` and `git.branch` keys: the branch as
//! the key's secondary line and, for `git.status`, whether the work tree
//! is clean.
//!
//! Read with the same `git status` as the actions (local only, no optional
//! locks), at most every [`STATE_INTERVAL`] while the key is shown.

use std::time::Duration;

use deckpilot_core::{ButtonState, Indicator};

use crate::porcelain::Status;

/// How often a shown key's repository is read at most.
pub const STATE_INTERVAL: Duration = Duration::from_secs(30);

/// The key state for `status`. With `changes`: an indicator — green when
/// clean, amber with uncommitted changes, red with conflicts.
pub fn state_of(status: &Status, changes: bool) -> ButtonState {
    let branch = &status.branch;
    let detail = match (&branch.name, &branch.head) {
        (Some(name), _) => name.clone(),
        (None, Some(head)) => format!("@{}", &head[..head.len().min(7)]),
        (None, None) => "no branch".to_owned(),
    };
    let state = ButtonState::new().with_detail(detail);
    if !changes {
        return state;
    }
    let indicator = if status.counts().conflicts > 0 {
        Indicator::Error
    } else if status.clean() {
        Indicator::Ok
    } else {
        Indicator::Warning
    };
    state.with_indicator(indicator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::porcelain::parse;

    fn z(lines: &[&str]) -> String {
        let mut out = lines.join("\0");
        out.push('\0');
        out
    }

    const OID: &str = "# branch.oid 1234567890abcdef1234567890abcdef12345678";

    #[test]
    fn clean_dirty_and_conflicted_work_trees() {
        let clean = parse(&z(&[OID, "# branch.head main"])).unwrap();
        assert_eq!(
            state_of(&clean, true),
            ButtonState::new()
                .with_detail("main")
                .with_indicator(Indicator::Ok)
        );
        let dirty = parse(&z(&[OID, "# branch.head feature/x", "? notes.txt"])).unwrap();
        assert_eq!(
            state_of(&dirty, true),
            ButtonState::new()
                .with_detail("feature/x")
                .with_indicator(Indicator::Warning)
        );
        let conflict = parse(&z(&[
            OID,
            "# branch.head main",
            "u UU N... 100644 100644 100644 100644 \
             1111111111111111111111111111111111111111 \
             2222222222222222222222222222222222222222 \
             3333333333333333333333333333333333333333 a.txt",
        ]))
        .unwrap();
        assert_eq!(state_of(&conflict, true).indicator, Some(Indicator::Error));
        // The branch key shows only the branch.
        assert_eq!(
            state_of(&dirty, false),
            ButtonState::new().with_detail("feature/x")
        );
    }

    #[test]
    fn detached_and_unborn_heads() {
        let detached = parse(&z(&[OID, "# branch.head (detached)"])).unwrap();
        assert_eq!(
            state_of(&detached, false).detail.as_deref(),
            Some("@1234567")
        );
        let unborn = parse(&z(&["# branch.oid (initial)", "# branch.head main"])).unwrap();
        assert_eq!(state_of(&unborn, false).detail.as_deref(), Some("main"));
    }

    /// The provider end to end, with a fake `git` program that records its
    /// arguments: no repository is read (Unix only).
    #[cfg(unix)]
    mod fake {
        use std::path::PathBuf;
        use std::sync::Arc;

        use deckpilot_core::launch::RecordingLauncher;
        use deckpilot_core::{
            ActionParameters, ActionRegistry, ExecutionContext, MachineConfig,
            MachineResourceBinding, Os, Profile, Project, ProjectKey, ResourceKey, ResourceKind,
            StateError,
        };

        use super::*;
        use crate::actions::{BRANCH, CHANGES, STATUS, id};

        struct Fake {
            dir: PathBuf,
            registry: ActionRegistry,
            profile: Profile,
            machine: MachineConfig,
        }

        impl Drop for Fake {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }

        fn fake(body: &str) -> Fake {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!(
                "deckpilot-git-state-test-{}",
                deckpilot_core::ProfileId::new()
            ));
            std::fs::create_dir_all(dir.join("shop")).unwrap();
            let program = dir.join("git");
            std::fs::write(
                &program,
                format!(
                    "#!/bin/sh\necho \"$@\" >> '{}'\n{body}\n",
                    dir.join("calls.log").display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = ActionParameters::new().with(
                crate::EXECUTABLE,
                deckpilot_core::ParameterValue::Text(program.display().to_string()),
            );
            let module = crate::GitModule::with_launcher(Arc::new(RecordingLauncher::new()));
            let mut registry = ActionRegistry::default();
            for handler in deckpilot_core::FeatureModule::action_handlers(&module, &config) {
                registry
                    .register_for_module(
                        crate::module_id(),
                        &deckpilot_core::FeatureModule::manifest(&module).capabilities,
                        handler,
                    )
                    .unwrap();
            }
            let mut profile = Profile::new("Work");
            let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
            project.resources.insert(
                "repository".into(),
                ResourceKey::new("shop.repository").unwrap(),
            );
            profile.projects.push(project);
            let mut machine = MachineConfig::new(Os::current());
            machine.set_binding(MachineResourceBinding {
                key: ResourceKey::new("shop.repository").unwrap(),
                kind: ResourceKind::Directory,
                value: dir.join("shop").display().to_string(),
            });
            Fake {
                dir,
                registry,
                profile,
                machine,
            }
        }

        impl Fake {
            fn state(&self, kind: &str) -> Option<Result<ButtonState, StateError>> {
                let context = ExecutionContext::new(
                    &self.profile,
                    Some(&self.profile.projects[0]),
                    &self.machine,
                );
                self.registry
                    .button_state(&id(kind), &ActionParameters::new(), &context)
            }

            fn calls(&self) -> String {
                std::fs::read_to_string(self.dir.join("calls.log")).unwrap_or_default()
            }
        }

        #[test]
        fn status_keys_read_the_project_repository() {
            let fake = fake(
                r"printf '# branch.oid 1234567890abcdef1234567890abcdef12345678\0# branch.head main\0? notes.txt\0'",
            );
            let state = fake.state(STATUS).unwrap().unwrap();
            assert_eq!(state.detail.as_deref(), Some("main"));
            assert_eq!(state.indicator, Some(Indicator::Warning));
            let calls = fake.calls();
            assert_eq!(calls.lines().count(), 1);
            assert!(calls.contains(&fake.dir.join("shop").display().to_string()));
            assert!(calls.contains("--no-optional-locks"));
            assert!(calls.contains("status --porcelain=v2 --branch -z --untracked-files=normal"));

            let branch = fake.state(BRANCH).unwrap().unwrap();
            assert_eq!(branch, ButtonState::new().with_detail("main"));
            assert!(fake.calls().contains("--untracked-files=no"));
            // Other Git keys have no live state.
            assert!(fake.state(CHANGES).is_none());
        }

        #[test]
        fn failures_are_error_states() {
            let fake = fake(
                "echo 'fatal: not a git repository (or any of the parent directories): .git' >&2; exit 128",
            );
            let error = fake.state(STATUS).unwrap().unwrap_err();
            assert_eq!(error.code, crate::codes::NOT_A_REPOSITORY);
        }
    }
}
