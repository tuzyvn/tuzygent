use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-env-changed=AGENTCTL_RELEASE_DATE");
    for path in ["HEAD", "refs", "packed-refs", "index"] {
        if let Some(path) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let mut commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    if Command::new("git")
        .args(["diff", "--quiet", "HEAD", "--", "."])
        .status()
        .is_ok_and(|status| status.code() == Some(1))
    {
        commit.push_str("-dirty");
    }
    // Release builds may supply an explicit date; local builds use the source commit date.
    let date = std::env::var("AGENTCTL_RELEASE_DATE")
        .ok()
        .filter(|date| !date.trim().is_empty())
        .or_else(|| git(&["log", "-1", "--format=%cs"]))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=AGENTCTL_COMMIT={commit}");
    println!("cargo:rustc-env=AGENTCTL_RELEASE_DATE={date}");
}
