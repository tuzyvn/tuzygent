#!/usr/bin/env python3
"""
Autonomous Continuous Evolution & Research Loop for Tuzygent.
Orchestrates:
  1. Probe & Benchmark
  2. Research Backlog (from IMPROVEMENT_PLAN.md & Harness Memory)
  3. Propose & Experiment in Isolated Branch/Worktree
  4. Verify & Gatekeep
  5. Merge & Self-Update via `agent-ctl update`
"""

import argparse
import os
import subprocess
import sys
import time
from pathlib import Path

from memory_bridge import find_lessons, load_lessons
from version_manager import get_current_version, release_and_push


def run_cmd(cmd, check=True):
    print(f"[$] {cmd}")
    res = subprocess.run(cmd, shell=True)
    if check and res.returncode != 0:
        raise RuntimeError(f"Command failed with exit code {res.returncode}: {cmd}")
    return res.returncode


def evaluate():
    return run_cmd("python3 harness/evaluators/benchmark.py", check=False) == 0


def trigger_self_update(summary=None, bump_type="patch", push=True, force_bump=False):
    print("\n🚀 [Update Gatekeeper] Bumping version, committing, building release & pushing to remote...")
    new_ver = release_and_push(
        summary=summary,
        bump_type=bump_type,
        push=push,
        force_bump=force_bump,
    )
    print(f"✅ System agent-ctl binary updated & pushed successfully (v{new_ver}).")


def print_evolution_status():
    lessons = load_lessons()
    print("==================================================")
    print("        TUZYGENT AUTONOMOUS RESEARCH STATUS       ")
    print("==================================================")
    print(f"• Active Harness Lessons in Memory: {len(lessons)}")
    for l in lessons[-5:]:
        print(f"  - [{l.get('id')}] ({l.get('node_type')}): {l.get('error_pattern')[:50]}...")
    print("• Baseline Assessment: IMPROVEMENT_PLAN.md & AUDIT_REPORT.md loaded.")
    print("==================================================")


def main():
    parser = argparse.ArgumentParser(description="Tuzygent Autonomous Evolution Harness")
    parser.add_argument("--benchmark", action="store_true", help="Run benchmark evaluation only")
    parser.add_argument("--update", action="store_true", help="Bump version, commit, update binary and push")
    parser.add_argument("--bump", choices=["patch", "minor", "major"], default="patch", help="Version bump level")
    parser.add_argument("--no-push", action="store_true", help="Skip git push after release")
    parser.add_argument("--force-bump", action="store_true", help="Force version bump even if no working tree diff")
    parser.add_argument("--loop", action="store_true", help="Run one autonomous research & improvement cycle")
    args = parser.parse_args()

    print_evolution_status()

    if args.benchmark:
        passed = evaluate()
        sys.exit(0 if passed else 1)

    if args.update:
        trigger_self_update(
            bump_type=args.bump,
            push=not args.no_push,
            force_bump=args.force_bump,
        )
        sys.exit(0)

    # Standard loop: Benchmark -> Gatekeep -> Bump + Commit + Build + Push
    passed = evaluate()
    if passed:
        print("\n🎉 All benchmarks and scenarios passed cleanly!")
        trigger_self_update(
            summary="verified benchmark & harness scenarios pass",
            bump_type=args.bump,
            push=not args.no_push,
            force_bump=args.force_bump,
        )
    else:
        print("\n⚠️ Baseline benchmark has failures. Check storage/harness_memory for recorded lessons.")
        sys.exit(1)


if __name__ == "__main__":
    main()
