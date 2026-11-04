#!/usr/bin/env python3
"""
Autonomous Researcher Module for Tuzygent.
Implements the Scientific Method Loop:
  1. Ingestion: Reads AUDIT_REPORT.md, IMPROVEMENT_PLAN.md, and harness_lessons.json.
  2. Hypothesis Generation: Formulates an isolated research goal.
  3. Dispatch: Triggers agent-ctl run with the configured research engine (antigravity/claude/codex).
  4. Verification: Benchmarks the outcome and records collective lessons.
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from memory_bridge import load_lessons

IMPROVEMENT_PLAN = Path("IMPROVEMENT_PLAN.md")
AUDIT_REPORT = Path("AUDIT_REPORT.md")
RESEARCH_BACKLOG = Path("RESEARCH_BACKLOG.md")


def extract_backlog_items():
    """Parse unsolved items from RESEARCH_BACKLOG.md, then fallback to IMPROVEMENT_PLAN.md."""
    items = []

    # Priority 1: Check dynamic RESEARCH_BACKLOG.md
    if RESEARCH_BACKLOG.exists():
        content = RESEARCH_BACKLOG.read_text(encoding="utf-8")
        matches = re.findall(r"^###\s+\[(P[0-2])\]\s+(TOPIC-[^:]+:\s+[^\n]+)", content, re.MULTILINE)
        for prio, title in matches:
            items.append(f"[{prio}] {title}")

    # If empty or not exists, auto-discover new topics!
    if not items:
        print("💡 Backlog trống, đang tự động quét mã nguồn để sinh đề tài mới...")
        subprocess.run(["python3", "harness/topic_generator.py"], check=False)
        if RESEARCH_BACKLOG.exists():
            content = RESEARCH_BACKLOG.read_text(encoding="utf-8")
            matches = re.findall(r"^###\s+\[(P[0-2])\]\s+(TOPIC-[^:]+:\s+[^\n]+)", content, re.MULTILINE)
            for prio, title in matches:
                items.append(f"[{prio}] {title}")

    # Fallback to static IMPROVEMENT_PLAN.md if still needed
    if not items and IMPROVEMENT_PLAN.exists():
        content = IMPROVEMENT_PLAN.read_text(encoding="utf-8")
        static_items = re.findall(r"^##\s+(\d+\.\s+P[0-2]\s+—\s+[^\n]+)", content, re.MULTILINE)
        items.extend(static_items)

    return items


def generate_research_mission(topic: str = None, engine: str = "antigravity") -> str:
    backlog = extract_backlog_items()
    lessons = load_lessons()

    if topic:
        selected_topic = topic
    elif backlog:
        selected_topic = backlog[0]
    else:
        raise RuntimeError("Không tìm thấy đề tài nghiên cứu nào trong Backlog và không còn lỗi code smell cần xử lý.")

    lessons_summary = ""
    if lessons:
        lessons_summary = "\n[Bài học kinh nghiệm từ quá khứ (Tránh lặp lại lỗi)]:\n"
        for l in lessons[-3:]:
            lessons_summary += f" - Lỗi '{l.get('error_pattern')[:60]}' -> Giải pháp: '{l.get('remediation_strategy')[:60]}'\n"

    mission = (
        f"MỤC TIÊU NGHIÊN CỨU: {selected_topic}\n"
        f"{lessons_summary}\n"
        "QUY TRÌNH THỰC HIỆN KHOA HỌC:\n"
        "1. Phân tích nguyên nhân gốc rễ và xác định crate liên quan trong crates/.\n"
        "2. TDD First: Viết 1 test case tái hiện lỗi/kịch bản trước.\n"
        "3. Tối ưu hoá mã nguồn để pass test mà không làm gãy các test cũ.\n"
        "4. Chạy `./scripts/run_harness.sh --benchmark` để xác minh không có hồi quy."
    )
    return mission


def run_research_session(topic: str = None, engine: str = "antigravity", sandbox: str = "host", dry_run: bool = False):
    print("==================================================")
    print("      TUZYGENT AUTONOMOUS RESEARCH DISPATCHER     ")
    print("==================================================")
    
    backlog = extract_backlog_items()
    print(f"📚 Phát hiện {len(backlog)} đề tài nghiên cứu từ Backlog:")
    for i, b in enumerate(backlog[:5], 1):
        print(f"  {i}. {b}")

    mission = generate_research_mission(topic, engine)
    print("\n🔬 [Nhiệm vụ nghiên cứu được khởi tạo]:")
    print("--------------------------------------------------")
    print(mission)
    print("--------------------------------------------------")

    if dry_run:
        print("\n[Dry-run] Chưa kích hoạt process. Sẵn sàng chạy với lệnh:")
        cmd = f'./target/release/agent-ctl run --agent-id "researcher-01" --goal "{mission}" --engine {engine} --sandbox {sandbox}'
        print(f"$ {cmd}")
        return 0

    print(f"\n🚀 Đang điều phối Agent nghiên cứu qua Engine '{engine}' (Sandbox: {sandbox})...")
    cmd = [
        "./target/release/agent-ctl",
        "run",
        "--agent-id", "researcher-01",
        "--goal", mission,
        "--engine", engine,
        "--sandbox", sandbox
    ]
    try:
        proc = subprocess.run(cmd)
        return proc.returncode
    except Exception as e:
        print(f"Lỗi khi điều phối agent: {e}")
        return 1


def main():
    parser = argparse.ArgumentParser(description="Tuzygent Autonomous Researcher")
    parser.add_argument("--topic", type=str, default=None, help="Chủ đề nghiên cứu cụ thể")
    parser.add_argument("--engine", type=str, default="antigravity", help="AI Engine (antigravity, claude, codex)")
    parser.add_argument("--sandbox", type=str, default="host", help="Sandbox driver (host, bubblewrap, docker)")
    parser.add_argument("--dry-run", action="store_true", help="Chỉ lập kế hoạch nghiên cứu, không spawn process")
    args = parser.parse_args()

    return run_research_session(topic=args.topic, engine=args.engine, sandbox=args.sandbox, dry_run=args.dry_run)


if __name__ == "__main__":
    sys.exit(main())
