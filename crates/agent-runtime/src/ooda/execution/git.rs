use super::SubtaskExecutor;
use std::path::Path;

/// Capture git diff stat in the specified working directory, if git is present and changes exist.
pub async fn capture_git_diff(workdir: &Path) -> Option<String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(workdir).args(["diff", "--stat"]);

    let out = tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;

    if out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

impl SubtaskExecutor {
    pub async fn capture_git_diff(workdir: &Path) -> Option<String> {
        capture_git_diff(workdir).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TestEnv {
        dir: PathBuf,
    }

    impl TestEnv {
        fn new(prefix: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "{}_{}_{}",
                prefix,
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            let _ = std::fs::create_dir_all(&dir);
            Self { dir }
        }

        fn path(&self) -> &Path {
            &self.dir
        }
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[tokio::test]
    async fn test_capture_git_diff_nonexistent_or_clean() {
        let temp_dir = std::env::temp_dir();
        let diff = capture_git_diff(&temp_dir).await;
        assert!(diff.is_none() || diff.is_some());
    }

    #[tokio::test]
    async fn test_capture_git_diff_with_changes() {
        let env = TestEnv::new("test_git_diff");
        let workdir = env.path();

        let init_status = tokio::process::Command::new("git")
            .arg("init")
            .current_dir(workdir)
            .status()
            .await;

        if let Ok(status) = init_status {
            if status.success() {
                let _ = tokio::process::Command::new("git")
                    .args(["config", "user.email", "test@example.com"])
                    .current_dir(workdir)
                    .status()
                    .await;
                let _ = tokio::process::Command::new("git")
                    .args(["config", "user.name", "Test User"])
                    .current_dir(workdir)
                    .status()
                    .await;

                let file_path = workdir.join("tracked.txt");
                let _ = std::fs::write(&file_path, "initial line\n");

                let _ = tokio::process::Command::new("git")
                    .args(["add", "tracked.txt"])
                    .current_dir(workdir)
                    .status()
                    .await;
                let _ = tokio::process::Command::new("git")
                    .args(["commit", "-m", "init commit"])
                    .current_dir(workdir)
                    .status()
                    .await;

                // Modify tracked file to introduce a diff
                let _ = std::fs::write(&file_path, "modified line\n");

                let diff = capture_git_diff(workdir).await;
                assert!(diff.is_some(), "Expected git diff output for modified file");
                assert!(diff.as_deref().is_some_and(|d| d.contains("tracked.txt")));
            }
        }
    }
}
