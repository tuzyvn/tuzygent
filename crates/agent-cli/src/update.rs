use anyhow::{bail, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const REPOSITORY: &str = "https://github.com/tuzyvn/tuzygent.git";

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(parent: &Path) -> Result<Self> {
        let path = parent.join(format!(".agent-ctl-update-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).context("Cannot create update staging directory")?;
        Ok(Self(path))
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn checked(command: &mut Command, description: &str) -> Result<()> {
    let status = command.status().with_context(|| description.to_owned())?;
    if !status.success() {
        bail!("{description}: {status}. Installed binary was not changed.");
    }
    Ok(())
}

/// Stage on the destination filesystem so the final rename is atomic on Unix.
fn install(candidate: &Path, destination: &Path, expected_commit: &str) -> Result<()> {
    let parent = destination
        .parent()
        .context("Executable has no parent directory")?;
    let staging = TemporaryDirectory::new(parent)?;
    let staged = staging.0.join("agent-ctl");
    fs::copy(candidate, &staged).context("Cannot stage the new executable")?;
    let output = Command::new(&staged)
        .arg("--version")
        .output()
        .context("Cannot run the new executable; installed binary was not changed")?;
    let version = String::from_utf8_lossy(&output.stdout);
    if !output.status.success()
        || !version.starts_with("agent-ctl ")
        || !version
            .lines()
            .any(|line| line == format!("Commit: {expected_commit}"))
    {
        bail!("New executable failed version validation; installed binary was not changed");
    }
    fs::File::open(&staged)?.sync_all()?;
    fs::rename(&staged, destination)
        .context("Cannot replace the installed executable; check directory write permissions")?;
    print!("Updated: {version}");
    Ok(())
}

pub fn run(check: bool) -> Result<()> {
    // Check/build everything in an isolated clone; never pull into the user's workspace.
    let temporary = TemporaryDirectory::new(&std::env::temp_dir())?;
    let source = temporary.0.join("source");
    println!("Checking {REPOSITORY} (main)...");
    checked(
        Command::new("git")
            .args([
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--single-branch",
                "--branch",
                "main",
                "--",
                REPOSITORY,
            ])
            .arg(&source),
        "Cannot fetch AgentCore; Git and network access are required",
    )?;
    let output = Command::new("git")
        .current_dir(&source)
        .args(["rev-parse", "--short", "HEAD"])
        .output()?;
    if !output.status.success() {
        bail!("Cannot read the fetched source commit");
    }
    let latest = String::from_utf8(output.stdout)?.trim().to_owned();
    let current = env!("AGENTCTL_COMMIT");
    println!("Installed: {current}\nLatest:    {latest}");
    if current == latest {
        println!("Already up to date.");
        return Ok(());
    }
    if check {
        println!("Update available. Run: agent-ctl update");
        return Ok(());
    }
    let destination = std::env::current_exe().context("Cannot locate the installed executable")?;
    // Fail before a lengthy build when the executable's directory is not writable.
    let _write_probe =
        TemporaryDirectory::new(destination.parent().context("Missing install directory")?)?;
    println!("Building {latest} in release mode (requires Cargo and a Rust toolchain)...");
    let target = temporary.0.join("target");
    checked(
        Command::new("cargo")
            .current_dir(&source)
            .args([
                "build",
                "--locked",
                "--release",
                "-p",
                "agent-cli",
                "--bin",
                "agent-ctl",
                "--target-dir",
            ])
            .arg(&target)
            .env_remove("CARGO_BUILD_TARGET")
            .env_remove("AGENTCTL_RELEASE_DATE"),
        "Cannot build AgentCore",
    )?;
    install(&target.join("release/agent-ctl"), &destination, &latest)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn validates_before_replacing_and_cleans_staging() {
        let directory = TemporaryDirectory::new(&std::env::temp_dir()).unwrap();
        let destination = directory.0.join("installed");
        let candidate = directory.0.join("candidate");
        fs::write(&destination, "old binary").unwrap();
        fs::write(
            &candidate,
            "#!/bin/sh\necho 'agent-ctl 0.1.0'\necho 'Commit: wrong'\n",
        )
        .unwrap();
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(install(&candidate, &destination, "abc1234").is_err());
        assert_eq!(fs::read_to_string(&destination).unwrap(), "old binary");
        fs::write(
            &candidate,
            "#!/bin/sh\necho 'agent-ctl 0.1.0'\necho 'Commit: abc1234'\n",
        )
        .unwrap();
        install(&candidate, &destination, "abc1234").unwrap();
        assert_eq!(
            fs::read(&destination).unwrap(),
            fs::read(&candidate).unwrap()
        );
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    }
}
