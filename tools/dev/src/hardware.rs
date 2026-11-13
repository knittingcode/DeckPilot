//! `deckpilot-dev hardware-test`: a reproducible test of a physical Stream
//! Deck with the real agent (`docs/release-validation.md`).
//!
//! It starts its own agent on an isolated data directory with a dedicated
//! configuration (two profiles whose keys are labelled `A1`… and `B1`…, each
//! key logging its press and release), then walks through discovery, every
//! key, images, brightness, profile and project switching, unplugging, an
//! agent restart and multiple devices. Steps that only a person can judge
//! ("do the keys show A1–A15?") are asked on the terminal. A report is
//! written next to the data.
//!
//! `--mock` runs the same procedure against a simulated MK.2 (presses are
//! simulated, visual questions answered with yes), which checks the
//! procedure itself; it is **not** a hardware test.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use deckpilot_core::{
    ActionParameters, BindingTarget, ControlBinding, DeviceLayout, LayoutStatus, Os,
    ParameterValue, Profile, Project, ProjectKey, Trigger,
};
use deckpilot_device::{
    ConnectionState, ControlId, ControlInput, DeviceId, DeviceModelId, ModelCatalog,
};
use deckpilot_ipc::{
    AgentClient, AgentEvent, AgentPaths, AgentStatus, ClientKind, KeyMaterial, Request, Response,
};
use deckpilot_modules::general;
use deckpilot_modules::projects::ProjectCatalog as _;
use deckpilot_storage::crypto::Password;
use deckpilot_storage::vault::Vault;
use deckpilot_storage::{ConfigStore as _, DatabaseKey, Encryption, SqliteStore};

const PASSWORD: &str = "deckpilot-hardware-test";
const MK2: &str = "elgato.streamdeck.mk2";

struct Options {
    dir: PathBuf,
    mock: bool,
    force: bool,
}

#[derive(Debug)]
enum Outcome {
    Passed(String),
    Failed(String),
    Skipped(String),
}

struct Report {
    steps: Vec<(String, Outcome)>,
}

impl Report {
    fn record(&mut self, step: &str, outcome: Outcome) {
        let (mark, detail) = match &outcome {
            Outcome::Passed(d) => ("PASS", d),
            Outcome::Failed(d) => ("FAIL", d),
            Outcome::Skipped(d) => ("SKIP", d),
        };
        println!("[{mark}] {step}: {detail}");
        self.steps.push((step.to_owned(), outcome));
    }

    fn failed(&self) -> bool {
        self.steps
            .iter()
            .any(|(_, outcome)| matches!(outcome, Outcome::Failed(_)))
    }

    fn write(&self, path: &Path, header: &str) -> std::io::Result<()> {
        let mut text = format!(
            "# Stream Deck hardware test\n\n{header}\n\n| Step | Result | Details |\n| ---- | ------ | ------- |\n"
        );
        for (step, outcome) in &self.steps {
            let (result, detail) = match outcome {
                Outcome::Passed(d) => ("passed", d),
                Outcome::Failed(d) => ("**failed**", d),
                Outcome::Skipped(d) => ("skipped", d),
            };
            let _ = writeln!(text, "| {step} | {result} | {detail} |");
        }
        std::fs::write(path, text)
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

/// A profile whose keys are labelled `<prefix><n>`; each key runs a log
/// action named like its label on press and `<label> released` on release.
fn test_profile(prefix: &str, keys: u16, catalog: &ModelCatalog) -> Result<Profile, String> {
    let registry = deckpilot_modules::builtin(Os::current()).action_registry();
    let mut profile = Profile::new(format!("Hardware test {prefix}"));
    for key in ["SHOP", "CMR"] {
        profile
            .add_project(Project::new(
                ProjectKey::new(key).map_err(|e| e.to_string())?,
                key,
            ))
            .map_err(|e| e.to_string())?;
    }
    let model = catalog
        .get(&DeviceModelId::new(MK2))
        .ok_or("the MK.2 model is missing from the catalog")?;
    let mut layout = DeviceLayout::new(format!("Hardware test {prefix}"), model.id.clone());
    let home = layout.home_page;
    let page = layout
        .pages
        .iter_mut()
        .find(|page| page.id == home)
        .ok_or("layout without a home page")?;
    for index in 0..keys {
        let label = format!("{prefix}{}", index + 1);
        for (trigger, name) in [
            (Trigger::Press, label.clone()),
            (Trigger::Release, format!("{label} released")),
        ] {
            let action = registry
                .create_instance(
                    &name,
                    general::log_id(),
                    &ActionParameters::new().with("message", ParameterValue::Text(name.clone())),
                )
                .map_err(|e| e.to_string())?;
            page.bindings.push(ControlBinding {
                control: ControlId::Key { index },
                trigger,
                target: BindingTarget::action(action.id),
                label: Some(label.clone()),
                appearance: None,
            });
            profile.actions.push(action);
        }
    }
    let layout_id = layout.id;
    profile.layouts.push(layout);
    profile
        .activate_layout(layout_id)
        .map_err(|e| e.to_string())?;
    let issues = profile.validate(catalog);
    if !issues.is_empty() {
        return Err(format!("test profile is invalid: {issues:?}"));
    }
    Ok(profile)
}

/// A new, password-protected configuration with the two test profiles.
fn prepare(dir: &Path) -> Result<(DatabaseKey, Profile, Profile), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let key = Vault::new(dir.join("deckpilot.db"))
        .initialize(&Password::new(PASSWORD))
        .map_err(|e| e.to_string())?;
    let store = SqliteStore::open(dir.join("deckpilot.db"), &Encryption::Key(key.clone()))
        .map_err(|e| e.to_string())?;
    let catalog = ModelCatalog::builtin();
    let a = test_profile("A", 15, &catalog)?;
    let b = test_profile("B", 15, &catalog)?;
    for profile in [&a, &b] {
        store.save_profile(profile).map_err(|e| e.to_string())?;
    }
    store
        .set_active_profile(Some(a.id))
        .map_err(|e| e.to_string())?;
    Ok((key, a, b))
}

struct Agent {
    child: Child,
    paths: AgentPaths,
}

impl Agent {
    fn start(root: &Path, dir: &Path, mock: bool) -> Result<Self, String> {
        let binary = root
            .join("target/debug")
            .join(format!("deckpilot-agent{}", std::env::consts::EXE_SUFFIX));
        if !binary.is_file() {
            return Err(format!(
                "{} is missing; run `cargo build -p deckpilot-agent` first",
                binary.display()
            ));
        }
        let mut command = Command::new(binary);
        command.arg("--headless");
        if mock {
            command.args(["--mock-devices", "mk2"]);
        }
        let log = std::fs::File::create(dir.join("agent-output.log")).map_err(|e| e.to_string())?;
        let child = command
            .env(deckpilot_ipc::paths::DATA_DIR_ENV, dir)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| e.to_string())?;
        let agent = Self {
            child,
            paths: AgentPaths::in_dir(dir),
        };
        agent.wait(Duration::from_secs(20), "the agent to start", |_| true)?;
        Ok(agent)
    }

    fn request(&self, request: Request) -> Result<Response, String> {
        runtime().block_on(async {
            let mut client = AgentClient::connect(&self.paths, ClientKind::Other)
                .await
                .map_err(|e| e.to_string())?;
            client.request(request).await.map_err(|e| e.to_string())
        })
    }

    fn status(&self) -> Result<AgentStatus, String> {
        match self.request(Request::GetStatus)? {
            Response::Status { status } => Ok(*status),
            other => Err(format!("unexpected {other:?}")),
        }
    }

    fn wait(
        &self,
        timeout: Duration,
        what: &str,
        condition: impl Fn(&AgentStatus) -> bool,
    ) -> Result<AgentStatus, String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(status) = self.status()
                && condition(&status)
            {
                return Ok(status);
            }
            if Instant::now() > deadline {
                return Err(format!("timed out waiting for {what}"));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn unlock(&self, key: &DatabaseKey) -> Result<(), String> {
        self.request(Request::Unlock {
            key: KeyMaterial::new(key.to_hex()),
        })
        .map(|_| ())
    }

    fn stop(mut self) {
        let _ = self.request(Request::Shutdown);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
    }
}

/// Asks a yes/no question; `--mock` answers yes.
fn confirm(question: &str, mock: bool) -> bool {
    if mock {
        println!("  {question} [y/n] y (simulated)");
        return true;
    }
    print!("  {question} [y/n] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok() && answer.trim().eq_ignore_ascii_case("y")
}

fn pause(instruction: &str, mock: bool) {
    if mock {
        return;
    }
    print!("  {instruction} — then press Enter ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

fn first_device(status: &AgentStatus) -> Option<&deckpilot_ipc::DeviceInfo> {
    status
        .devices
        .iter()
        .find(|d| d.state == ConnectionState::Connected)
}

fn shows_layout(status: &AgentStatus, profile: &Profile) -> bool {
    let layout = profile.layouts[0].id;
    status.devices.iter().any(|d| {
        d.state == ConnectionState::Connected
            && matches!(&d.layout, LayoutStatus::Compatible { layout: l, .. } if *l == layout)
    })
}

/// Collects press and release events of every key; `--mock` simulates them.
fn keys(agent: &Agent, device: &DeviceId, mock: bool, keys: u16) -> Result<String, String> {
    let wanted: BTreeSet<String> = (1..=keys)
        .flat_map(|n| [format!("A{n}"), format!("A{n} released")])
        .collect();
    let paths = agent.paths.clone();
    let device = device.clone();
    runtime().block_on(async move {
        let mut events = AgentClient::connect(&paths, ClientKind::Other)
            .await
            .map_err(|e| e.to_string())?;
        events.subscribe().await.map_err(|e| e.to_string())?;
        if mock {
            let mut client = AgentClient::connect(&paths, ClientKind::Other)
                .await
                .map_err(|e| e.to_string())?;
            for index in 0..keys {
                for input in [ControlInput::Press, ControlInput::Release] {
                    client
                        .request(Request::SimulateInput {
                            device: device.clone(),
                            control: ControlId::Key { index },
                            input,
                        })
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        } else {
            println!(
                "  Press and release every key once, in the order of their labels: A1, A2, … A{keys} (2 minutes)."
            );
        }
        let mut seen = BTreeSet::new();
        // Labels of the pressed keys, in the order they were pressed.
        let mut presses = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        while !wanted.is_subset(&seen) {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, events.next_event()).await {
                Ok(Ok(AgentEvent::ActionCompleted { name, .. })) => {
                    if wanted.contains(&name) && seen.insert(name.clone()) {
                        if !name.ends_with(" released") {
                            presses.push(name.clone());
                        }
                        if !mock {
                            println!("    {name}");
                        }
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => return Err(error.to_string()),
                Err(_) => {
                    let missing: Vec<_> = wanted.difference(&seen).cloned().collect();
                    return Err(format!("no event for: {}", missing.join(", ")));
                }
            }
        }
        match press_order_problem(&presses, keys) {
            Some(problem) => Err(problem),
            None => Ok(format!(
                "{keys} keys, press and release each; the key showing each label reports that key"
            )),
        }
    })
}

/// Checks that keys pressed in the order of their labels (A1, A2, …)
/// reported the same order. A difference means that the key showing a
/// label sends another key's index: images and input disagree about the
/// key order (rotation or mirroring).
fn press_order_problem(presses: &[String], keys: u16) -> Option<String> {
    let expected: Vec<String> = (1..=keys).map(|n| format!("A{n}")).collect();
    let position = presses
        .iter()
        .zip(&expected)
        .position(|(got, want)| got != want)?;
    Some(format!(
        "pressed in the order of the labels, the device reported {} instead of {} (press {}); \
         images and key input disagree, or a key was pressed out of order: run the test again",
        presses[position],
        expected[position],
        position + 1
    ))
}

#[allow(clippy::too_many_lines)]
fn run(root: &Path, options: &Options, report: &mut Report) -> Result<(), String> {
    let (key, a, b) = prepare(&options.dir)?;
    let agent = Agent::start(root, &options.dir, options.mock)?;
    agent.unlock(&key)?;

    // 1–2. Discovery and model.
    let status = match agent.wait(Duration::from_secs(30), "a connected Stream Deck", |s| {
        first_device(s).is_some()
    }) {
        Ok(status) => status,
        Err(error) => {
            report.record("Device discovery", Outcome::Failed(error));
            agent.stop();
            return Ok(());
        }
    };
    let device = first_device(&status).ok_or("device vanished")?.clone();
    report.record(
        "Device discovery",
        Outcome::Passed(format!(
            "{} {} via {}",
            device.model_name, device.id.serial, device.source
        )),
    );
    let mk2 = device.id.model.as_str() == MK2;
    report.record(
        "Model detection",
        if mk2 {
            Outcome::Passed("Stream Deck MK.2".into())
        } else {
            Outcome::Failed(format!(
                "detected {} (the test layout is for the MK.2)",
                device.id.model
            ))
        },
    );
    if !mk2 {
        agent.stop();
        return Ok(());
    }

    // 3–5. Layout, images and text, keys.
    match agent.wait(Duration::from_secs(10), "the test layout", |s| {
        shows_layout(s, &a)
    }) {
        Ok(_) => report.record(
            "Layout applied",
            Outcome::Passed("profile A layout is active".into()),
        ),
        Err(error) => report.record("Layout applied", Outcome::Failed(error)),
    }
    report.record(
        "Key images and text",
        if confirm(
            "Do the keys show A1 … A15 left to right, top to bottom (A1 top left, A5 top right, A15 bottom right), upright and not mirrored?",
            options.mock,
        ) {
            Outcome::Passed("confirmed".into())
        } else {
            Outcome::Failed("not as expected".into())
        },
    );
    report.record(
        "Key press and release",
        match keys(&agent, &device.id, options.mock, 15) {
            Ok(detail) => Outcome::Passed(detail),
            Err(error) => Outcome::Failed(error),
        },
    );

    // 6. Brightness.
    let mut brightness_ok = true;
    for (percent, question) in [
        (10, "Is the device dim now?"),
        (100, "Is it at full brightness now?"),
    ] {
        agent.request(Request::SetBrightness { percent })?;
        let applied = agent.status()?.brightness == percent;
        brightness_ok &= applied && confirm(question, options.mock);
    }
    agent.request(Request::SetBrightness { percent: 60 })?;
    report.record(
        "Brightness",
        if brightness_ok {
            Outcome::Passed("10 %, 100 %, back to 60 %".into())
        } else {
            Outcome::Failed("brightness did not follow".into())
        },
    );

    // 7. Profile switching.
    agent.request(Request::SetActiveProfile { profile: b.id })?;
    let switched = agent
        .wait(Duration::from_secs(10), "profile B", |s| {
            shows_layout(s, &b)
        })
        .is_ok()
        && confirm("Do the keys show B1 … B15 now?", options.mock);
    agent.request(Request::SetActiveProfile { profile: a.id })?;
    let back = agent
        .wait(Duration::from_secs(10), "profile A", |s| {
            shows_layout(s, &a)
        })
        .is_ok();
    report.record(
        "Profile switching",
        if switched && back {
            Outcome::Passed("A → B → A".into())
        } else {
            Outcome::Failed("the device did not follow the profile".into())
        },
    );

    // 8. Project switching.
    let cmr = a.projects[1].id;
    agent.request(Request::SelectProject { project: cmr })?;
    report.record(
        "Project switching",
        match agent.wait(Duration::from_secs(5), "project CMR", |s| {
            s.active_project.as_ref().is_some_and(|p| p.id == cmr)
        }) {
            Ok(_) => Outcome::Passed("SHOP → CMR".into()),
            Err(error) => Outcome::Failed(error),
        },
    );

    // 9. Unplug and replug (a USB communication loss).
    if options.mock {
        report.record(
            "Unplug and replug",
            Outcome::Skipped(
                "needs a physical device (simulated in the agent's integration tests)".into(),
            ),
        );
    } else {
        pause("Unplug the Stream Deck", false);
        let gone = agent
            .wait(Duration::from_secs(20), "the device to disconnect", |s| {
                !s.devices
                    .iter()
                    .any(|d| d.id == device.id && d.state == ConnectionState::Connected)
            })
            .is_ok();
        pause("Plug it in again", false);
        let back = agent
            .wait(Duration::from_secs(20), "the device to return", |s| {
                shows_layout(s, &a)
            })
            .is_ok()
            && confirm("Do the keys show A1 … A15 again?", false);
        report.record(
            "Unplug and replug",
            if gone && back {
                Outcome::Passed("disconnected, reconnected with the same layout".into())
            } else {
                Outcome::Failed("the agent did not recover".into())
            },
        );
    }

    // 10. Agent restart: keys are blanked when it stops; profile, project
    // and brightness are restored when it starts.
    agent.stop();
    report.record(
        "Keys cleared on stop",
        if confirm("The agent has stopped: are all keys black?", options.mock) {
            Outcome::Passed("confirmed".into())
        } else {
            Outcome::Failed("images remained after the agent stopped".into())
        },
    );
    let agent = Agent::start(root, &options.dir, options.mock)?;
    agent.unlock(&key)?;
    let restored = agent.wait(
        Duration::from_secs(30),
        "the device after the restart",
        |s| {
            shows_layout(s, &a)
                && s.active_project.as_ref().is_some_and(|p| p.id == cmr)
                && s.brightness == 60
        },
    );
    report.record(
        "Agent restart",
        match restored {
            Ok(_)
                if confirm(
                    "After the restart, do the keys show A1 … A15?",
                    options.mock,
                ) =>
            {
                Outcome::Passed("layout, project CMR and brightness 60 % restored".into())
            }
            Ok(_) => Outcome::Failed("images not as expected".into()),
            Err(error) => Outcome::Failed(error),
        },
    );

    // 11. Without the configuration app.
    report.record(
        "Agent without the configuration app",
        Outcome::Passed("the whole test ran with the agent alone (no app process)".into()),
    );

    // 12. Multiple devices.
    let connected: Vec<String> = agent
        .status()?
        .devices
        .iter()
        .filter(|d| d.state == ConnectionState::Connected)
        .map(|d| format!("{} {}", d.model_name, d.id.serial))
        .collect();
    report.record(
        "Multiple devices",
        if connected.len() > 1 {
            Outcome::Passed(connected.join(", "))
        } else {
            Outcome::Skipped(format!("one device connected ({})", connected.join(", ")))
        },
    );
    agent.stop();
    Ok(())
}

/// Other programs that may hold the device.
fn competing_processes() -> Vec<String> {
    let output = Command::new(if cfg!(windows) { "tasklist" } else { "ps" })
        .args(if cfg!(windows) {
            vec![]
        } else {
            vec!["-axo", "comm"]
        })
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| {
            let line = line.to_lowercase();
            line.contains("deckpilot-agent")
                || line.contains("stream deck")
                || line.contains("streamdeck")
        })
        .map(|line| line.trim().to_owned())
        .collect()
}

pub fn command(root: &Path, args: &[String]) -> Result<(), String> {
    let flag = |name: &str| args.iter().any(|a| a == name);
    let dir = args
        .iter()
        .position(|a| a == "--dir")
        .and_then(|i| args.get(i + 1))
        .map_or_else(|| root.join(".dev/hardware-test"), PathBuf::from);
    let options = Options {
        dir,
        mock: flag("--mock"),
        force: flag("--force"),
    };
    if !options.mock {
        let others = competing_processes();
        if !others.is_empty() && !options.force {
            return Err(format!(
                "other programs may be using the Stream Deck ({}); quit them (e.g. `make down`, the Elgato app) or pass --force",
                others.join(", ")
            ));
        }
    }
    let mut report = Report { steps: Vec::new() };
    let result = run(root, &options, &mut report);
    let header = format!(
        "DeckPilot {}, {} {}, {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if options.mock {
            "**simulated MK.2 (procedure check, not a hardware test)**"
        } else {
            "physical device"
        }
    );
    let path = options.dir.join("report.md");
    let _ = report.write(&path, &header);
    println!("report: {}", path.display());
    result?;
    if report.failed() {
        Err("hardware test failed".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::press_order_problem;

    fn labels(numbers: &[u16]) -> Vec<String> {
        numbers.iter().map(|n| format!("A{n}")).collect()
    }

    #[test]
    fn presses_in_label_order_pass() {
        let all: Vec<u16> = (1..=15).collect();
        assert_eq!(press_order_problem(&labels(&all), 15), None);
    }

    #[test]
    fn a_mirrored_or_rotated_key_order_is_reported() {
        // Rows mirrored: the key showing A1 reports index 4 (A5).
        let mirrored = [5, 4, 3, 2, 1, 10, 9, 8, 7, 6, 15, 14, 13, 12, 11];
        let problem = press_order_problem(&labels(&mirrored), 15).unwrap();
        assert!(problem.contains("reported A5 instead of A1"), "{problem}");

        let rotated: Vec<u16> = (1..=15).rev().collect();
        let problem = press_order_problem(&labels(&rotated), 15).unwrap();
        assert!(problem.contains("reported A15 instead of A1"), "{problem}");
    }
}
