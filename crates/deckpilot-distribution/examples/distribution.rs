//! Prints the distribution record of the running build, and the agent
//! executable override it honours, as `key=value` lines.
//! `tests/release_build.rs` builds this in the release and debug profiles.

fn main() {
    let record = deckpilot_distribution::current();
    println!("distribution_channel={}", record.distribution_channel());
    println!("update_provider={}", record.update_provider());
    println!(
        "package_format={}",
        record
            .package_format()
            .map_or("", deckpilot_distribution::PackageFormat::name)
    );
    println!("release_channel={}", record.release_channel());
    println!("application_version={}", record.application_version());
    println!("platform={}", record.platform());
    println!("architecture={}", record.architecture());
    println!("development_override={}", record.is_development_override());
    println!(
        "agent_executable_override={}",
        deckpilot_distribution::agent_executable_override()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
}
