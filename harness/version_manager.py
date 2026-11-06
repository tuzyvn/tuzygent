#!/usr/bin/env python3
"""
Tuzygent Workspace Version Manager & Automated Git Release Pipeline.
Responsibilities:
  1. Parse and bump semantic version (patch / minor / major) in root Cargo.toml.
  2. Synchronize workspace member versions in Cargo.lock so `--locked --offline` builds succeed.
  3. Stage, commit, build release `agent-ctl` (clean commit hash without `-dirty`), install to `~/.local/bin/agent-ctl`, and push to remote origin.
"""

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Optional, Tuple

ROOT_DIR = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT_DIR / "Cargo.toml"
CARGO_LOCK = ROOT_DIR / "Cargo.lock"

WORKSPACE_CRATE_NAMES = (
    "agent-cli",
    "agent-core",
    "agent-domain",
    "agent-engine",
    "agent-memory",
    "agent-runtime",
    "agent-server",
)


def get_current_version() -> str:
    """Read current [workspace.package] version from root Cargo.toml."""
    content = CARGO_TOML.read_text(encoding="utf-8")
    match = re.search(
        r'(\[workspace\.package\][\s\S]*?^\s*version\s*=\s*")(\d+\.\d+\.\d+)(")',
        content,
        re.MULTILINE,
    )
    if not match:
        raise RuntimeError("Could not find [workspace.package] version in Cargo.toml")
    return match.group(2)


def compute_next_version(current: str, bump_type: str = "patch") -> str:
    """Compute next semantic version (patch, minor, or major)."""
    parts = [int(x) for x in current.split(".")]
    if len(parts) != 3:
        raise ValueError(f"Invalid semantic version: {current}")
    major, minor, patch = parts
    if bump_type == "major":
        return f"{major + 1}.0.0"
    if bump_type == "minor":
        return f"{major}.{minor + 1}.0"
    return f"{major}.{minor}.{patch + 1}"


def bump_version_files(bump_type: str = "patch") -> Tuple[str, str]:
    """
    Increment version in root Cargo.toml and synchronize workspace crates in Cargo.lock.
    Returns (old_version, new_version).
    """
    old_version = get_current_version()
    new_version = compute_next_version(old_version, bump_type)

    # 1. Update Cargo.toml
    toml_content = CARGO_TOML.read_text(encoding="utf-8")
    new_toml_content, count = re.subn(
        r'(\[workspace\.package\][\s\S]*?^\s*version\s*=\s*")(\d+\.\d+\.\d+)(")',
        rf"\g<1>{new_version}\g<3>",
        toml_content,
        count=1,
        flags=re.MULTILINE,
    )
    if count == 0:
        raise RuntimeError("Failed to update version in Cargo.toml")
    CARGO_TOML.write_text(new_toml_content, encoding="utf-8")

    # 2. Update workspace packages in Cargo.lock so `--locked --offline` stays valid
    if CARGO_LOCK.exists():
        lock_content = CARGO_LOCK.read_text(encoding="utf-8")
        names_pattern = "|".join(re.escape(n) for n in WORKSPACE_CRATE_NAMES)
        lock_pattern = rf'(name\s*=\s*"(?:{names_pattern})"\nversion\s*=\s*")(\d+\.\d+\.\d+)(")'
        new_lock_content = re.sub(lock_pattern, rf"\g<1>{new_version}\g<3>", lock_content)
        CARGO_LOCK.write_text(new_lock_content, encoding="utf-8")

    return old_version, new_version


def has_git_changes() -> bool:
    """Return True if the git working tree or index has modifications or untracked files."""
    res = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=ROOT_DIR,
        capture_output=True,
        text=True,
        check=False,
    )
    return bool(res.stdout.strip())


def get_current_branch() -> str:
    """Return current git branch name (defaults to 'main')."""
    res = subprocess.run(
        ["git", "rev-parse", "--abbrev-ref", "HEAD"],
        cwd=ROOT_DIR,
        capture_output=True,
        text=True,
        check=False,
    )
    branch = res.stdout.strip()
    return branch if branch and branch != "HEAD" else "main"


def release_and_push(
    summary: Optional[str] = None,
    bump_type: str = "patch",
    push: bool = True,
    force_bump: bool = False,
) -> str:
    """
    Full release lifecycle:
      1. Bump semantic version in Cargo.toml & Cargo.lock (if working tree changed or force_bump=True).
      2. Commit all changes first so `build.rs` embeds a clean commit hash (no `-dirty`) and new version.
      3. Build release `agent-ctl` and install to `~/.local/bin/agent-ctl`.
      4. Push commit to remote origin.
    """
    changed = has_git_changes()
    if changed or force_bump:
        old_ver, new_ver = bump_version_files(bump_type)
        print(f"🔖 [Version Manager] Đã nâng cấp phiên bản: v{old_ver} -> v{new_ver}")
    else:
        new_ver = get_current_version()
        print(f"ℹ️ [Version Manager] Không có thay đổi mã nguồn mới (giữ nguyên v{new_ver}).")

    # Stage & commit before building release binary so commit hash is clean (no `-dirty`)
    if has_git_changes():
        subprocess.run(["git", "add", "-A"], cwd=ROOT_DIR, check=True)
        commit_msg = (
            f"chore(release): v{new_ver} - {summary}"
            if summary
            else f"chore(release): bump version to v{new_ver}"
        )
        print(f"📦 [Git Commit] Đang commit mã nguồn: {commit_msg}")
        subprocess.run(["git", "commit", "-m", commit_msg], cwd=ROOT_DIR, check=True)

    # Build release binary AFTER commit so it embeds the exact release commit & version
    print(f"🔨 [Release Build] Đang biên dịch agent-ctl v{new_ver} (clean commit)...")
    subprocess.run(
        ["cargo", "build", "--release", "-p", "agent-cli", "--bin", "agent-ctl"],
        cwd=ROOT_DIR,
        check=True,
    )

    target_bin = ROOT_DIR / "target" / "release" / "agent-ctl"
    install_dir = Path.home() / ".local" / "bin"
    install_dir.mkdir(parents=True, exist_ok=True)
    install_path = install_dir / "agent-ctl"
    shutil.copy2(target_bin, install_path)
    subprocess.run([str(install_path), "--version"], cwd=ROOT_DIR, check=True)
    print(f"✅ [System Update] Đã cập nhật binary hệ thống tại {install_path} (v{new_ver}).")

    if push:
        branch = get_current_branch()
        print(f"🚀 [Git Push] Đang đẩy code và phiên bản v{new_ver} lên origin/{branch}...")
        push_res = subprocess.run(
            ["git", "push", "origin", branch],
            cwd=ROOT_DIR,
            check=False,
        )
        if push_res.returncode == 0:
            print(f"✅ [Git Push] Đã push thành công v{new_ver} lên origin/{branch}!")
        else:
            print(f"⚠️ [Git Push] Không thể push lên origin/{branch} (exit code {push_res.returncode}).")

    return new_ver


def main() -> int:
    parser = argparse.ArgumentParser(description="Tuzygent Version Bump & Git Push Manager")
    parser.add_argument(
        "--bump",
        choices=["patch", "minor", "major"],
        default="patch",
        help="Loại phiên bản cần tăng (mặc định: patch)",
    )
    parser.add_argument(
        "--message",
        "-m",
        type=str,
        default=None,
        help="Nội dung tóm tắt cho commit release",
    )
    parser.add_argument(
        "--no-push",
        action="store_true",
        help="Chỉ tăng version, commit và cập nhật binary local mà không git push",
    )
    parser.add_argument(
        "--force-bump",
        action="store_true",
        help="Luôn tăng version ngay cả khi chưa có file thay đổi",
    )
    args = parser.parse_args()

    release_and_push(
        summary=args.message,
        bump_type=args.bump,
        push=not args.no_push,
        force_bump=args.force_bump,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
