//! Docker is never touched: execution tests run a fake `docker` script that
//! records its arguments and answers like Docker would (Unix only); the
//! command lines themselves are tested in `cli` on every platform.

use std::sync::Arc;

use deckpilot_core::ActionParameters;

use super::*;

fn settings(config: &ActionParameters) -> Arc<DockerSettings> {
    Arc::new(crate::settings(config))
}

fn handler(kind: &str, settings: &Arc<DockerSettings>) -> Box<dyn ActionHandler> {
    handlers(settings)
        .into_iter()
        .find(|h| h.definition().id.as_str() == kind)
        .unwrap()
}

#[test]
fn every_action_type_declares_its_effect() {
    let all = handlers(&settings(&ActionParameters::new()));
    let ids: Vec<_> = all.iter().map(|h| h.definition().id.to_string()).collect();
    assert_eq!(
        ids,
        [
            STATUS,
            CONTAINERS,
            COMPOSE_STATUS,
            COMPOSE_UP,
            COMPOSE_DOWN,
            COMPOSE_RESTART,
            LOGS
        ]
    );
    for handler in &all {
        let definition = handler.definition();
        let modifies = matches!(
            definition.id.as_str(),
            COMPOSE_UP | COMPOSE_DOWN | COMPOSE_RESTART
        );
        assert_eq!(
            definition.node.effect == ActionEffect::Modifies,
            modifies,
            "{}",
            definition.id
        );
        assert!(definition.node.cancellable);
        assert!(definition.node.outputs.iter().any(|o| o.key == CONTEXT));
        // Nothing offers removing volumes or images.
        assert!(
            !definition
                .parameters
                .iter()
                .any(|p| p.key.contains("olume") || p.key.contains("mage")),
            "{}",
            definition.id
        );
    }
    let logs = handler(LOGS, &settings(&ActionParameters::new()));
    assert!(
        logs.definition()
            .node
            .outputs
            .iter()
            .any(|o| o.key == "logs" && o.sensitive)
    );
}

#[test]
fn names_are_checked_when_saving() {
    let handler = handler(CONTAINERS, &settings(&ActionParameters::new()));
    let profile = deckpilot_core::Profile::new("Work");
    let bad = ActionParameters::new().with(CONTEXT, ParameterValue::Text("--host=evil".into()));
    assert_eq!(
        handler.check(&bad, &profile),
        ["“--host=evil” is not a valid context name"]
    );
    let foreign = ActionParameters::new().with(
        PROJECT,
        ParameterValue::Project(deckpilot_core::ProjectId::new()),
    );
    assert_eq!(
        handler.check(&foreign, &profile),
        ["the project does not belong to this profile"]
    );
}

#[cfg(unix)]
mod fake {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use deckpilot_core::{
        CancellationToken, ExecutionContext, ExecutionControl, MachineConfig,
        MachineResourceBinding, Os, Profile, Progress, Project, ProjectKey, ResourceKey,
        ResourceKind, ResourceRef,
    };

    use super::*;

    /// How the fake engine behaves.
    #[derive(Clone, Copy, PartialEq)]
    enum Engine {
        Running,
        Down,
        Forbidden,
        /// `compose up` hangs, to test cancellation.
        Slow,
    }

    struct Fake {
        dir: PathBuf,
        profile: Profile,
        machine: MachineConfig,
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn script(log: &Path, engine: Engine) -> String {
        let version = match engine {
            Engine::Running | Engine::Slow => {
                r#"echo '{"Client":{"Version":"29.1.0","Os":"linux"},"Server":{"Version":"29.1.0","Os":"linux"}}'"#
                    .to_owned()
            }
            Engine::Down => r#"echo '{"Client":{"Version":"29.1.0","Os":"linux"},"Server":null}'; echo 'Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?' >&2; exit 1"#.to_owned(),
            Engine::Forbidden => r#"echo '{"Client":{"Version":"29.1.0"},"Server":null}'; echo 'permission denied while trying to connect to the Docker daemon socket' >&2; exit 1"#.to_owned(),
        };
        let up = if engine == Engine::Slow {
            "sleep 30"
        } else {
            "echo 'Container shop-web-1 Started' >&2"
        };
        format!(
            r#"#!/bin/sh
printf '%s\n' "$@" >> '{log}'
echo '@@end@@' >> '{log}'
[ "$1" = "--context" ] && shift 2
case "$1" in
  context) echo fake-ctx ;;
  version) {version} ;;
  ps) echo '{{"Names":"web-1","Image":"nginx","State":"running","Status":"Up 1 hour","Labels":"token=abc"}}'
      echo '{{"Names":"job-1","Image":"busybox","State":"exited","Status":"Exited (0)"}}' ;;
  logs) echo 'out line'; echo 'err line' >&2 ;;
  compose)
    case " $* " in
      *" version --short "*) echo 2.30.0 ;;
      *" ps "*) echo '{{"Name":"shop-web-1","Service":"web","State":"running","Status":"Up"}}'
                echo '{{"Name":"shop-db-1","Service":"db","State":"exited","Status":"Exited (1)"}}' ;;
      *" up "*) {up} ;;
      *" down "*) echo 'Container shop-web-1 Removed' >&2 ;;
      *" restart "*) echo 'Container shop-web-1 Restarted' >&2 ;;
      *" logs "*) echo 'web  | hello' ;;
    esac ;;
esac
"#,
            log = log.display()
        )
    }

    fn fake(engine: Engine) -> Fake {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "deckpilot-docker-test-{}",
            deckpilot_core::ProfileId::new()
        ));
        std::fs::create_dir_all(dir.join("shop")).unwrap();
        std::fs::write(dir.join("shop/compose.dev.yaml"), "services: {}\n").unwrap();
        let docker = dir.join("docker");
        std::fs::write(&docker, script(&dir.join("calls.log"), engine)).unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut profile = Profile::new("Work");
        let mut project = Project::new(ProjectKey::new("SHOP").unwrap(), "Shop");
        project
            .resources
            .insert("compose".into(), ResourceKey::new("shop.compose").unwrap());
        profile.projects.push(project);
        let mut machine = MachineConfig::new(Os::current());
        machine.set_binding(MachineResourceBinding {
            key: ResourceKey::new("shop.compose").unwrap(),
            kind: ResourceKind::DockerEnvironment,
            value: dir.join("shop").display().to_string(),
        });
        machine.set_binding(MachineResourceBinding {
            key: ResourceKey::new("shop.compose-file").unwrap(),
            kind: ResourceKind::Other,
            value: dir.join("shop/compose.dev.yaml").display().to_string(),
        });
        Fake {
            dir,
            profile,
            machine,
        }
    }

    impl Fake {
        fn settings(&self, extra: &[(&str, ParameterValue)]) -> Arc<DockerSettings> {
            let mut config = ActionParameters::new().with(
                crate::EXECUTABLE,
                ParameterValue::Text(self.dir.join("docker").display().to_string()),
            );
            for (key, value) in extra {
                config = config.with(*key, value.clone());
            }
            settings(&config)
        }

        fn allowed(&self) -> Arc<DockerSettings> {
            self.settings(&[(crate::ALLOW_STATE_CHANGES, ParameterValue::Boolean(true))])
        }

        fn context(&self) -> ExecutionContext<'_> {
            ExecutionContext::new(
                &self.profile,
                Some(&self.profile.projects[0]),
                &self.machine,
            )
        }

        fn run(
            &self,
            kind: &str,
            settings: &Arc<DockerSettings>,
            parameters: &ActionParameters,
        ) -> ActionResult {
            self.run_in(kind, settings, parameters, &self.context())
        }

        #[allow(clippy::unused_self)] // Reads like `run`.
        fn run_in(
            &self,
            kind: &str,
            settings: &Arc<DockerSettings>,
            parameters: &ActionParameters,
            context: &ExecutionContext<'_>,
        ) -> ActionResult {
            let handler = handler(kind, settings);
            let normalized = handler.definition().normalize(parameters).unwrap();
            handler.execute(&normalized, context)
        }

        /// The argument lists Docker was called with.
        fn calls(&self) -> Vec<Vec<String>> {
            std::fs::read_to_string(self.dir.join("calls.log"))
                .unwrap_or_default()
                .split("@@end@@\n")
                .filter(|c| !c.is_empty())
                .map(|c| c.lines().map(str::to_owned).collect())
                .collect()
        }
    }

    fn none() -> ActionParameters {
        ActionParameters::new()
    }

    #[test]
    fn availability_is_an_answer_not_a_failure() {
        let running = fake(Engine::Running);
        let status = running.run(STATUS, &running.settings(&[]), &none());
        assert!(status.is_success(), "{}", status.message);
        assert_eq!(status.data["available"], "true");
        assert_eq!(status.data[CONTEXT], "fake-ctx");
        assert_eq!(status.data["serverVersion"], "29.1.0");
        assert_eq!(status.data["composeVersion"], "2.30.0");

        for (engine, reason) in [
            (Engine::Down, codes::DAEMON_UNAVAILABLE),
            (Engine::Forbidden, codes::PERMISSION_DENIED),
        ] {
            let fake = fake(engine);
            let status = fake.run(STATUS, &fake.settings(&[]), &none());
            assert!(status.is_success());
            assert_eq!(status.data["available"], "false");
            assert_eq!(status.data["reason"], reason);
            assert_eq!(status.data["clientVersion"], "29.1.0");
        }

        let missing = settings(&ActionParameters::new().with(
            crate::EXECUTABLE,
            ParameterValue::Text("/nonexistent/deckpilot/docker".into()),
        ));
        let status = running.run(STATUS, &missing, &none());
        assert_eq!(status.data["available"], "false");
        assert_eq!(status.data["reason"], codes::DOCKER_NOT_FOUND);
        // Other actions fail with the reason (the fake lists containers
        // even then; a real `docker ps` would fail like `version`).
        let forbidden = fake(Engine::Forbidden);
        let missing_docker = forbidden.run(CONTAINERS, &missing, &none());
        assert_eq!(
            missing_docker.code.as_deref(),
            Some(codes::DOCKER_NOT_FOUND)
        );
    }

    #[test]
    fn every_command_names_its_context() {
        let fake = fake(Engine::Running);
        // Docker's current context, asked for first.
        let listed = fake.run(CONTAINERS, &fake.settings(&[]), &none());
        assert!(listed.is_success(), "{}", listed.message);
        assert_eq!(listed.data[CONTEXT], "fake-ctx");
        // The module's context, then the action's, win.
        let module = fake.settings(&[(crate::CONTEXT, ParameterValue::Text("remote-prod".into()))]);
        let configured = fake.run(CONTAINERS, &module, &none());
        assert_eq!(configured.data[CONTEXT], "remote-prod");
        let explicit = fake.run(
            CONTAINERS,
            &module,
            &none().with(CONTEXT, ParameterValue::Text("staging".into())),
        );
        assert_eq!(explicit.data[CONTEXT], "staging");
        let calls = fake.calls();
        assert_eq!(calls[0], ["context", "show"]);
        for call in &calls[1..] {
            assert_eq!(call[0], "--context", "{call:?}");
        }
        assert_eq!(calls[2][1], "remote-prod");
        assert_eq!(calls[3][1], "staging");
    }

    #[test]
    fn containers_are_listed_without_their_labels() {
        let fake = fake(Engine::Running);
        let listed = fake.run(
            CONTAINERS,
            &fake.settings(&[]),
            &none()
                .with("all", ParameterValue::Boolean(true))
                .with("name", ParameterValue::Text("web".into())),
        );
        assert_eq!(listed.data["count"], "2");
        assert_eq!(listed.data["running"], "1");
        assert_eq!(listed.data["names"], "web-1,job-1");
        assert!(!listed.data["containers"].contains("token"));
        let call = fake.calls().pop().unwrap();
        assert!(call.contains(&"--all".to_owned()) && call.contains(&"name=web".to_owned()));
    }

    #[test]
    fn compose_projects_come_from_resources() {
        let fake = fake(Engine::Running);
        let status = fake.run(COMPOSE_STATUS, &fake.settings(&[]), &none());
        assert!(status.is_success(), "{}", status.message);
        assert_eq!(status.data["services"], "2");
        assert_eq!(status.data["running"], "1");
        assert_eq!(status.data["allRunning"], "false");
        assert_eq!(status.data["projectKey"], "SHOP");
        let call = fake.calls().pop().unwrap();
        let directory = fake.dir.join("shop").display().to_string();
        assert_eq!(
            call[2..5],
            [
                "compose".to_owned(),
                "--project-directory".into(),
                directory
            ]
        );

        // A resource naming the file, and a project name.
        let by_file = fake.run(
            COMPOSE_STATUS,
            &fake.settings(&[]),
            &none()
                .with(
                    COMPOSE,
                    ParameterValue::Resource(ResourceRef::Fixed {
                        key: ResourceKey::new("shop.compose-file").unwrap(),
                    }),
                )
                .with("projectName", ParameterValue::Text("shop_dev".into())),
        );
        assert!(by_file.is_success(), "{}", by_file.message);
        assert_eq!(by_file.data["project"], "shop_dev");
        let call = fake.calls().pop().unwrap();
        assert!(
            call.windows(2)
                .any(|w| w[0] == "-f" && w[1].ends_with("compose.dev.yaml"))
        );
        assert!(call.windows(2).any(|w| w[0] == "-p" && w[1] == "shop_dev"));

        let no_project = ExecutionContext::new(&fake.profile, None, &fake.machine);
        let refused = fake.run_in(COMPOSE_STATUS, &fake.settings(&[]), &none(), &no_project);
        assert_eq!(refused.code.as_deref(), Some("no-active-project"));
    }

    #[test]
    fn changes_must_be_allowed_and_never_remove_volumes() {
        let fake = fake(Engine::Running);
        for kind in [COMPOSE_UP, COMPOSE_DOWN, COMPOSE_RESTART] {
            let refused = fake.run(kind, &fake.settings(&[]), &none());
            assert_eq!(
                refused.code.as_deref(),
                Some(codes::CHANGES_DISABLED),
                "{kind}"
            );
        }
        assert_eq!(fake.calls().len(), 0, "nothing ran");

        let services = none().with("services", ParameterValue::Text("web\n\ndb\n".into()));
        let up = fake.run(COMPOSE_UP, &fake.allowed(), &services);
        assert!(up.is_success(), "{}", up.message);
        assert!(up.data["output"].contains("Started"));
        let call = fake.calls().pop().unwrap();
        assert!(call.ends_with(&[
            "up".into(),
            "--detach".into(),
            "--".into(),
            "web".into(),
            "db".into()
        ]));

        let down = fake.run(
            COMPOSE_DOWN,
            &fake.allowed(),
            &none().with("removeOrphans", ParameterValue::Boolean(true)),
        );
        assert!(down.is_success(), "{}", down.message);
        assert!(down.message.contains("volumes kept"));
        let call = fake.calls().pop().unwrap();
        assert!(call.ends_with(&["down".into(), "--remove-orphans".into()]));
        assert!(
            !call
                .iter()
                .any(|a| a == "-v" || a == "--volumes" || a == "--rmi")
        );

        let restart = fake.run(COMPOSE_RESTART, &fake.allowed(), &none());
        assert!(restart.is_success());

        let bad = fake.run(
            COMPOSE_UP,
            &fake.allowed(),
            &none().with(
                "services",
                ParameterValue::Text("web\n--force-recreate".into()),
            ),
        );
        assert_eq!(bad.code.as_deref(), Some(codes::INVALID_NAME));
    }

    #[test]
    fn changes_stop_when_cancelled() {
        let fake = fake(Engine::Slow);
        let token = CancellationToken::new();
        let progress = |_: Progress| {};
        let control = ExecutionControl::new(&token, &progress);
        let context = fake.context().with_control(&control);
        let canceller = {
            let token = token.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                token.cancel();
            })
        };
        let begun = Instant::now();
        let result = fake.run_in(COMPOSE_UP, &fake.allowed(), &none(), &context);
        canceller.join().unwrap();
        assert_eq!(
            result.code.as_deref(),
            Some("cancelled"),
            "{}",
            result.message
        );
        assert!(begun.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn logs_are_bounded_and_need_one_target() {
        let fake = fake(Engine::Running);
        let container = fake.run(
            LOGS,
            &fake.settings(&[]),
            &none()
                .with("container", ParameterValue::Text("web-1".into()))
                .with("lines", ParameterValue::Integer(20))
                .with("since", ParameterValue::Text("10m".into())),
        );
        assert!(container.is_success(), "{}", container.message);
        assert_eq!(container.data["logs"], "out line\nerr line");
        assert_eq!(container.data["lineCount"], "2");
        let call = fake.calls().pop().unwrap();
        assert!(call.ends_with(&[
            "logs".into(),
            "--tail".into(),
            "20".into(),
            "--since".into(),
            "10m".into(),
            "--".into(),
            "web-1".into()
        ]));
        assert!(!call.iter().any(|a| a == "-f" || a == "--follow"));

        let service = fake.run(
            LOGS,
            &fake.settings(&[]),
            &none().with("service", ParameterValue::Text("web".into())),
        );
        assert_eq!(service.data["logs"], "web  | hello");
        assert_eq!(service.data["projectKey"], "SHOP");

        let neither = fake.run(LOGS, &fake.settings(&[]), &none());
        assert_eq!(neither.code.as_deref(), Some("invalid-parameters"));
    }
}
