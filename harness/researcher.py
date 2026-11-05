#!/usr/bin/env python3
"""
Autonomous Continuous Researcher Engine for Tuzygent.
Features:
  - Continuous Loop Mode: Automatically resolves topic N -> gates & verifies -> moves to N+1.
  - Circuit Breaker: Automatically detects stuck topics and skips to next after retries.
  - Auto-Gatekeeper: Runs benchmarks upon topic completion, commits, and self-updates agent-ctl.
  - Infinite Evolution: Re-scans codebase AST when backlog is depleted.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import List, Dict, Optional, Tuple

sys.path.insert(0, str(Path(__file__).resolve().parent))
from memory_bridge import load_lessons, record_lesson

RESEARCH_BACKLOG = Path("RESEARCH_BACKLOG.md")
IMPROVEMENT_PLAN = Path("IMPROVEMENT_PLAN.md")


def extract_backlog_items() -> List[Dict]:
    """Parse pending (unsolved) items from RESEARCH_BACKLOG.md."""
    items = []

    if RESEARCH_BACKLOG.exists():
        content = RESEARCH_BACKLOG.read_text(encoding="utf-8")
        blocks = re.split(r"\n(?=###\s+\[)", content)
        for block in blocks:
            if not block.strip().startswith("### ["):
                continue
            if "COMPLETED" in block or "BLOCKED" in block:
                continue
            match = re.search(r"^###\s+\[(P[0-2])\]\s+((TOPIC-[^:]+):\s+[^\n]+)", block, re.MULTILINE)
            if match:
                prio, title, topic_id = match.group(1), match.group(2), match.group(3)
                items.append({
                    "display": f"[{prio}] {title}",
                    "topic_id": topic_id,
                    "title": title,
                    "priority": prio,
                })

    # If empty or all completed, auto-discover and brainstorm new topics!
    if not items:
        print("\n💡 Không còn đề tài pending. Đang tự động quét mã nguồn và kích hoạt Feature Brainstorming Engine...")
        subprocess.run(["python3", "harness/topic_generator.py"], check=False)
        if RESEARCH_BACKLOG.exists():
            content = RESEARCH_BACKLOG.read_text(encoding="utf-8")
            blocks = re.split(r"\n(?=###\s+\[)", content)
            for block in blocks:
                if not block.strip().startswith("### ["):
                    continue
                if "COMPLETED" in block or "BLOCKED" in block:
                    continue
                match = re.search(r"^###\s+\[(P[0-2])\]\s+((TOPIC-[^:]+):\s+[^\n]+)", block, re.MULTILINE)
                if match:
                    prio, title, topic_id = match.group(1), match.group(2), match.group(3)
                    items.append({
                        "display": f"[{prio}] {title}",
                        "topic_id": topic_id,
                        "title": title,
                        "priority": prio,
                    })

    # If still empty, force brainstorm mode
    if not items:
        print("💡 Kích hoạt ép buộc Brainstorming mở rộng tính năng mới (--brainstorm)...")
        subprocess.run(["python3", "harness/topic_generator.py", "--brainstorm"], check=False)
        if RESEARCH_BACKLOG.exists():
            content = RESEARCH_BACKLOG.read_text(encoding="utf-8")
            blocks = re.split(r"\n(?=###\s+\[)", content)
            for block in blocks:
                if not block.strip().startswith("### ["):
                    continue
                if "COMPLETED" in block or "BLOCKED" in block:
                    continue
                match = re.search(r"^###\s+\[(P[0-2])\]\s+((TOPIC-[^:]+):\s+[^\n]+)", block, re.MULTILINE)
                if match:
                    prio, title, topic_id = match.group(1), match.group(2), match.group(3)
                    items.append({
                        "display": f"[{prio}] {title}",
                        "topic_id": topic_id,
                        "title": title,
                        "priority": prio,
                    })

    # Always prioritize P0 (Urgent Bugfixes) first, then P1, then P2
    prio_order = {"P0": 0, "P1": 1, "P2": 2}
    items.sort(key=lambda x: prio_order.get(x.get("priority", "P1"), 9))

    return items


def mark_topic_completed(topic_id: str):
    """Mark a topic as COMPLETED in RESEARCH_BACKLOG.md."""
    if not RESEARCH_BACKLOG.exists():
        return
    content = RESEARCH_BACKLOG.read_text(encoding="utf-8")
    now_str = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")

    pattern = rf"(###\s+\[P[0-2]\]\s+{re.escape(topic_id)}:[^\n]+\n(?:[^\n]+\n)*?-\s+\*\*Trạng thái\*\*:\s*)`PENDING_RESEARCH`\s*⏳"
    replacement = rf"\1`COMPLETED` ✅ ({now_str})"
    new_content, count = re.subn(pattern, replacement, content)
    if count > 0:
        RESEARCH_BACKLOG.write_text(new_content, encoding="utf-8")
        print(f"\n🎉 [Tiến độ] Đã đánh dấu hoàn thành đề tài {topic_id} trong {RESEARCH_BACKLOG.name}!")


def mark_topic_blocked(topic_id: str, reason: str):
    """Mark a topic as BLOCKED to prevent infinite failure loops."""
    if not RESEARCH_BACKLOG.exists():
        return
    content = RESEARCH_BACKLOG.read_text(encoding="utf-8")
    now_str = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")

    pattern = rf"(###\s+\[P[0-2]\]\s+{re.escape(topic_id)}:[^\n]+\n(?:[^\n]+\n)*?-\s+\*\*Trạng thái\*\*:\s*)`PENDING_RESEARCH`\s*⏳"
    replacement = rf"\1`BLOCKED` ⚠️ ({now_str} - {reason})"
    new_content, count = re.subn(pattern, replacement, content)
    if count > 0:
        RESEARCH_BACKLOG.write_text(new_content, encoding="utf-8")
        print(f"\n⚠️ [Circuit Breaker] Đã tạm bỏ qua đề tài {topic_id} do lỗi lặp lại. Tiến sang đề tài tiếp theo...")


def generate_research_mission(topic_entry: Dict, engine: str = "antigravity") -> str:
    lessons = load_lessons()
    lessons_summary = ""
    if lessons:
        lessons_summary = "\n[Bài học kinh nghiệm từ quá khứ (Tránh lặp lại lỗi)]:\n"
        for l in lessons[-3:]:
            lessons_summary += f" - Lỗi '{l.get('error_pattern')[:60]}' -> Giải pháp: '{l.get('remediation_strategy')[:60]}'\n"

    mission = (
        f"MỤC TIÊU NGHIÊN CỨU: {topic_entry['display']}\n"
        f"{lessons_summary}\n"
        "QUY TRÌNH THỰC HIỆN KHOA HỌC:\n"
        "1. Phân tích nguyên nhân gốc rễ và xác định crate liên quan trong crates/.\n"
        "2. TDD First: Viết 1 test case tái hiện lỗi/kịch bản trước.\n"
        "3. Tối ưu hoá mã nguồn để pass test mà không làm gãy các test cũ.\n"
        "4. Chạy `./scripts/run_harness.sh --benchmark` để xác minh không có hồi quy."
    )
    return mission


def run_continuous_research_loop(
    topic_filter: Optional[str] = None,
    engine: str = "antigravity",
    sandbox: str = "host",
    continuous: bool = False,
    max_topics: int = 1,
    dry_run: bool = False,
):
    print("==================================================")
    print("   TUZYGENT AUTONOMOUS CONTINUOUS RESEARCH LOOP   ")
    print("==================================================")
    print(f"• Mode: {'VÒNG LẶP LIÊN TỤC (CONTINUOUS)' if continuous else f'ĐƠN LẺ ({max_topics} đề tài)'}")
    print(f"• Engine: {engine} | Sandbox: {sandbox}")
    print("==================================================")

    agent_ctl_bin = Path("./target/release/agent-ctl")
    if not dry_run and not agent_ctl_bin.exists():
        print("🔨 Đang tự động biên dịch agent-ctl release binary...")
        subprocess.run(["cargo", "build", "--release", "-p", "agent-cli", "--bin", "agent-ctl"], check=True)

    completed_count = 0
    failure_tracker: Dict[str, int] = {}
    total_target = 999999 if continuous else max_topics

    try:
        while completed_count < total_target:
            backlog = extract_backlog_items()
            if not backlog:
                print("\n🏆 Toàn bộ đề tài đã hoàn tất và mã nguồn đạt chuẩn 100%! Dừng vòng lặp.")
                break

            # Filter or select top pending
            if topic_filter:
                candidates = [b for b in backlog if topic_filter.lower() in b["display"].lower()]
                if not candidates:
                    print(f"Không tìm thấy đề tài nào khớp với bộ lọc: {topic_filter}")
                    break
                current_topic = candidates[0]
            else:
                current_topic = backlog[0]

            topic_id = current_topic["topic_id"]
            print(f"\n──────────────────────────────────────────────────")
            print(f"▶ [Nhiệm vụ {completed_count + 1}/{'∞' if continuous else total_target}] Bắt đầu xử lý: {current_topic['display']}")
            print(f"──────────────────────────────────────────────────")

            mission = generate_research_mission(current_topic, engine)

            if dry_run:
                print(f"[Dry-run] Lệnh sẽ thực thi:\n$ ./target/release/agent-ctl run --agent-id researcher-01 --goal \"{mission[:80]}...\" --engine {engine} --sandbox {sandbox}")
                completed_count += 1
                if continuous and completed_count >= 3:
                    print("\n[Dry-run] Đã mô phỏng xong 3 đề tài liên tục. Dừng dry-run.")
                    break
                continue

            cmd = [
                "./target/release/agent-ctl",
                "run",
                "--agent-id", "researcher-01",
                "--goal", mission,
                "--engine", engine,
                "--sandbox", sandbox,
                "--auto-approve",
            ]

            proc = subprocess.run(cmd)
            if proc.returncode == 0:
                print(f"\n✅ Đề tài {topic_id} đã hoàn thành xuất sắc!")
                mark_topic_completed(topic_id)
                failure_tracker.pop(topic_id, None)
                completed_count += 1

                # Gatekeeper Verification & Self-Update
                print("\n🔍 Đang kích hoạt Gatekeeper đo lường toàn diện sau khi hoàn thành đề tài...")
                bench_proc = subprocess.run(["python3", "harness/evaluators/benchmark.py"])
                if bench_proc.returncode == 0:
                    print("🚀 Benchmark đạt chuẩn 100%! Tự động build và cập nhật agent-ctl...")
                    subprocess.run(["cargo", "build", "--release", "-p", "agent-cli", "--bin", "agent-ctl"], check=False)
                    subprocess.run(["cp", "target/release/agent-ctl", str(Path.home() / ".local/bin/agent-ctl")], check=False)
                else:
                    print("\n🚨 [Gatekeeper] Benchmark phát hiện lỗi hồi quy! Đang tự động nạp đề tài P0 sửa lỗi ngay...")
                    record_lesson(
                        node_type="gatekeeper_benchmark",
                        identifier=topic_id,
                        error_pattern="Benchmark tests or clippy failed after mission completion",
                        remediation_strategy="Chạy cargo test và cargo clippy để sửa triệt để lỗi hồi quy",
                    )
                    subprocess.run(["python3", "harness/topic_generator.py"], check=False)

                if completed_count < total_target:
                    print("\n⏳ Nghỉ 3 giây trước khi tự động tiến sang đề tài tiếp theo...")
                    time.sleep(3)
            else:
                fails = failure_tracker.get(topic_id, 0) + 1
                failure_tracker[topic_id] = fails
                print(f"\n⚠️ Đề tài {topic_id} chưa đạt chuẩn (lần {fails}/2).")
                record_lesson(
                    node_type="research_mission",
                    identifier=topic_id,
                    error_pattern=f"Mission exited with non-zero code {proc.returncode}",
                    remediation_strategy="Kiểm tra lại compilation errors và logic test",
                )
                if fails >= 2:
                    print(f"\n🚨 [Self-Healing Protocol] Đề tài {topic_id} gặp lỗi lặp lại ({fails} lần)! Tự động chuyển sang chế độ FIX LỖI...")
                    mark_topic_blocked(topic_id, f"Thất bại {fails} lần liên tiếp - chuyển sang P0 Bugfix")
                    # Tự động quét và nạp ngay đề tài P0 sửa lỗi hệ thống lặp lại vào Backlog!
                    subprocess.run(["python3", "harness/topic_generator.py"], check=False)
                    time.sleep(2)
                else:
                    time.sleep(3)

    except KeyboardInterrupt:
        print(f"\n\n🛑 Nhận tín hiệu dừng (Ctrl+C). Đã hoàn thành {completed_count} đề tài.")

    print(f"\n==================================================")
    print(f"🏁 TỔNG KẾT: Đã hoàn thành {completed_count} đề tài trong phiên làm việc.")
    print(f"==================================================")


def main():
    parser = argparse.ArgumentParser(description="Tuzygent Continuous Researcher")
    parser.add_argument("--topic", type=str, default=None, help="Chủ đề cụ thể cần làm")
    parser.add_argument("--engine", type=str, default="antigravity", help="AI Engine (antigravity, claude, codex)")
    parser.add_argument("--sandbox", type=str, default="host", help="Sandbox driver (host, bubblewrap, docker)")
    parser.add_argument("--continuous", "-c", action="store_true", help="Chạy liên tục không ngừng cho đến khi hết backlog")
    parser.add_argument("--max-topics", "-m", type=int, default=1, help="Số lượng đề tài tối đa cần giải quyết trong phiên (mặc định: 1)")
    parser.add_argument("--dry-run", action="store_true", help="Chỉ lập kế hoạch, không thực thi process")
    args = parser.parse_args()

    max_t = 999999 if args.continuous else args.max_topics
    run_continuous_research_loop(
        topic_filter=args.topic,
        engine=args.engine,
        sandbox=args.sandbox,
        continuous=args.continuous,
        max_topics=max_t,
        dry_run=args.dry_run,
    )


if __name__ == "__main__":
    main()
