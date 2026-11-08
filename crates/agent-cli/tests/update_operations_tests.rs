use std::fs;
use std::path::Path;
use std::process::Command;

use agent_cli::update::{ResilientUpdater, TemporaryDirectory};
use agent_cli::{CliExecutionError, CliFaultPolicy};

fn git<I, S>(directory: &Path, args: I) -> std::process::Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("git should be installed for this test")
}

fn init_repository(path: &Path, branch: &str) -> String {
    fs::create_dir_all(path).unwrap();
    let init = git(path, ["init", "--quiet", "--initial-branch", branch]);
    assert!(init.status.success(), "git init failed: {:?}", init);

    assert!(git(path, ["config", "user.name", "Test User"])
        .status
        .success());
    assert!(git(path, ["config", "user.email", "test@example.invalid"])
        .status
        .success());
    fs::write(path.join("README.md"), "local update fixture\n").unwrap();
    assert!(git(path, ["add", "README.md"]).status.success());
    assert!(git(path, ["commit", "--quiet", "-m", "fixture"])
        .status
        .success());

    let output = git(path, ["rev-parse", "--short", "HEAD"]);
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn fetch_repository_clones_selected_branch_and_replaces_existing_destination() {
    let temp = TemporaryDirectory::new(&std::env::temp_dir()).unwrap();
    let repository = temp.path().join("repository");
    let expected_commit = init_repository(&repository, "release/test");
    let destination = temp.path().join("source");
    fs::create_dir_all(&destination).unwrap();
    fs::write(destination.join("stale.txt"), "stale content").unwrap();

    let mut updater = ResilientUpdater::with_policy(CliFaultPolicy::none(), 0)
        .with_repository(repository.to_string_lossy().into_owned())
        .with_branch("release/test");

    updater.fetch_repository(&destination).unwrap();

    assert!(!destination.join("stale.txt").exists());
    assert_eq!(
        fs::read_to_string(destination.join("README.md")).unwrap(),
        "local update fixture\n"
    );
    let actual_branch = git(&destination, ["branch", "--show-current"]);
    assert!(actual_branch.status.success());
    assert_eq!(
        String::from_utf8(actual_branch.stdout).unwrap().trim(),
        "release/test"
    );
    let actual_commit = git(&destination, ["rev-parse", "--short", "HEAD"]);
    assert!(actual_commit.status.success());
    assert_eq!(
        String::from_utf8(actual_commit.stdout).unwrap().trim(),
        expected_commit
    );
}

#[test]
fn fetch_repository_returns_error_when_clone_fails() {
    let temp = TemporaryDirectory::new(&std::env::temp_dir()).unwrap();
    let destination = temp.path().join("source");
    let mut updater = ResilientUpdater::with_policy(CliFaultPolicy::none(), 0)
        .with_repository(
            temp.path()
                .join("missing-repository")
                .to_string_lossy()
                .into_owned(),
        )
        .with_branch("main");

    let error = updater.fetch_repository(&destination).unwrap_err();

    assert!(
        matches!(error, CliExecutionError::NonZeroExit { .. }),
        "expected clone failure to be reported as a non-zero Git exit, got {error:?}"
    );
}

#[test]
fn check_remote_commit_returns_short_head_commit() {
    let temp = TemporaryDirectory::new(&std::env::temp_dir()).unwrap();
    let repository = temp.path().join("repository");
    let expected_commit = init_repository(&repository, "main");
    let mut updater = ResilientUpdater::with_policy(CliFaultPolicy::none(), 0);

    let actual_commit = updater.check_remote_commit(&repository).unwrap();

    assert_eq!(actual_commit, expected_commit);
}

#[test]
fn check_remote_commit_returns_error_for_non_repository_directory() {
    let temp = TemporaryDirectory::new(&std::env::temp_dir()).unwrap();
    let source = temp.path().join("not-a-repository");
    fs::create_dir_all(&source).unwrap();
    let mut updater = ResilientUpdater::with_policy(CliFaultPolicy::none(), 0);

    let error = updater.check_remote_commit(&source).unwrap_err();

    assert!(
        matches!(error, CliExecutionError::NonZeroExit { .. }),
        "expected an unreadable commit to be reported as a non-zero Git exit, got {error:?}"
    );
}
