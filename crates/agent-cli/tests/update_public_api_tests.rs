use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

use agent_cli::update::{run, run_resilient_command, TemporaryDirectory, UpdateStatus};

static PATH_LOCK: Mutex<()> = Mutex::new(());

struct PathRestore(Option<std::ffi::OsString>);

impl Drop for PathRestore {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            env::set_var("PATH", path);
        } else {
            env::remove_var("PATH");
        }
    }
}

fn install_failing_git() -> (TemporaryDirectory, PathRestore) {
    let temp =
        TemporaryDirectory::new(&env::temp_dir()).expect("temporary directory should be created");
    let git = temp.path().join("git");
    fs::write(
        &git,
        "#!/bin/sh\necho 'simulated fetch failure' >&2\nexit 1\n",
    )
    .expect("fake git should be written");
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755))
        .expect("fake git should be executable");

    let old_path = env::var_os("PATH");
    env::set_var("PATH", temp.path());
    (temp, PathRestore(old_path))
}

#[test]
fn run_check_mode_succeeds_when_latest_commit_is_unavailable() {
    let _lock = PATH_LOCK.lock().expect("PATH lock should not be poisoned");
    let (_temp, _restore) = install_failing_git();

    let result = run(true);

    assert!(
        result.is_ok(),
        "check mode should report unavailable remote metadata without failing: {result:?}"
    );
}

#[test]
fn resilient_command_check_reports_latest_unavailable() {
    let _lock = PATH_LOCK.lock().expect("PATH lock should not be poisoned");
    let (_temp, _restore) = install_failing_git();

    let result = futures::executor::block_on(run_resilient_command(true));

    assert_eq!(
        result,
        Ok(UpdateStatus::LatestUnavailable {
            current: env!("AGENTCTL_COMMIT").to_string(),
        })
    );
}
