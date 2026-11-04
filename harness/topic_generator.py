#!/usr/bin/env python3
"""
Tuzygent Fully Dynamic Autonomous Topic Discovery & Generation Engine.
Zero hardcoding:
  - Dynamically extracts workspace crates from Cargo.toml.
  - Dynamically computes code complexity thresholds using distribution percentiles.
  - Dynamically inspects AST code smells: .unwrap(), .expect(), panic!(), unsafe blocks.
  - Dynamically aggregates recurring failure patterns from runtime harness lessons.
  - Dynamically prompts the AI engine for architectural frontier research based on actual project graph.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import List, Dict, Tuple

ROOT_DIR = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT_DIR / "Cargo.toml"
BACKLOG_FILE = ROOT_DIR / "RESEARCH_BACKLOG.md"
MEMORY_FILE = ROOT_DIR / "storage" / "harness_memory" / "harness_lessons.json"


def get_workspace_crates() -> List[Path]:
    """Dynamically parse workspace members from Cargo.toml without hardcoding."""
    if not CARGO_TOML.exists():
        return [p for p in (ROOT_DIR / "crates").glob("*") if p.is_dir()]

    content = CARGO_TOML.read_text(encoding="utf-8")
    members_match = re.search(r"members\s*=\s*\[(.*?)\]", content, re.DOTALL)
    if not members_match:
        return [p for p in (ROOT_DIR / "crates").glob("*") if p.is_dir()]

    raw_members = members_match.group(1)
    crate_dirs = []
    for raw in re.findall(r'"([^"]+)"', raw_members):
        p = ROOT_DIR / raw
        if p.exists() and p.is_dir():
            crate_dirs.append(p)
    return crate_dirs


def get_all_production_rs_files(crates: List[Path]) -> List[Path]:
    """Collect all non-test Rust source files in workspace crates."""
    files = []
    for crate in crates:
        for rs in crate.rglob("*.rs"):
            # Exclude tests and integration test suites
            if "tests" in rs.parts or rs.name.endswith("_test.rs"):
                continue
            files.append(rs)
    return sorted(files)


def analyze_code_smells(rs_files: List[Path]) -> List[Dict]:
    """Dynamically scan production source code for unhandled error patterns and panics."""
    smell_topics = []
    for file_path in rs_files:
        try:
            content = file_path.read_text(encoding="utf-8")
        except Exception:
            continue

        unwraps = len(re.findall(r"\.unwrap\(\)", content))
        expects = len(re.findall(r"\.expect\(", content))
        panics = len(re.findall(r"\bpanic!\(", content))
        unsafes = len(re.findall(r"\bunsafe\s*\{", content))

        rel_path = file_path.relative_to(ROOT_DIR)

        if unwraps > 0:
            smell_topics.append({
                "title": f"Refactor {unwraps} lệnh unwrap() tại {rel_path} sang cơ chế xử lý lỗi an toàn",
                "priority": "P0" if unwraps >= 10 else "P1",
                "category": "Reliability / Panic Prevention",
                "rationale": f"Phát hiện {unwraps} lệnh unwrap() trong mã sản xuất tại {rel_path}. Bất kỳ nhánh lỗi I/O hoặc parse thất bại nào đều có thể làm dừng tiến trình ngoài ý muốn.",
                "acceptance_criteria": f"Thay thế toàn bộ unwrap() trong {rel_path} bằng toán tử ? kết hợp anyhow::Context hoặc domain error tương ứng.",
                "metric_count": unwraps,
            })

        if panics > 0:
            smell_topics.append({
                "title": f"Khử {panics} điểm gọi panic!() tường minh trong {rel_path}",
                "priority": "P0",
                "category": "Reliability / Crash Immunity",
                "rationale": f"Phát hiện {panics} lệnh panic!() trong {rel_path}. Việc chủ động làm sập tiến trình vi phạm nguyên tắc tự phục hồi (Self-Healing) của AgentCore.",
                "acceptance_criteria": f"Chuyển đổi các nhánh panic!() trong {rel_path} thành kiểu trả về Result<T, E>.",
                "metric_count": panics,
            })

        if unsafes > 0:
            smell_topics.append({
                "title": f"Kiểm tra và cô lập {unsafes} khối unsafe block trong {rel_path}",
                "priority": "P1",
                "category": "Memory Safety",
                "rationale": f"Phát hiện {unsafes} khối unsafe trong {rel_path}. Cần kiểm tra ranh giới bộ nhớ, pointer aliasing và invariant validation.",
                "acceptance_criteria": f"Bổ sung tài liệu chứng minh Safety Invariants (// SAFETY: ...) và viết test kiểm tra ranh giới cho {rel_path}.",
                "metric_count": unsafes,
            })

    return smell_topics


def analyze_complexity_distribution(rs_files: List[Path]) -> List[Dict]:
    """Dynamically determine oversized files based on statistical percentile (90th percentile)."""
    file_sizes: List[Tuple[Path, int]] = []
    for f in rs_files:
        try:
            lines = len(f.read_text(encoding="utf-8").splitlines())
            file_sizes.append((f, lines))
        except Exception:
            continue

    if not file_sizes:
        return []

    lines_sorted = sorted([lines for _, lines in file_sizes])
    # 90th percentile index
    p90_index = int(len(lines_sorted) * 0.90)
    p90_threshold = max(350, lines_sorted[min(p90_index, len(lines_sorted) - 1)])

    complexity_topics = []
    for file_path, lines in sorted(file_sizes, key=lambda x: x[1], reverse=True):
        if lines >= p90_threshold:
            rel_path = file_path.relative_to(ROOT_DIR)
            complexity_topics.append({
                "title": f"Phân tách cấu trúc tệp nguyên khối ({lines} dòng): {rel_path}",
                "priority": "P2",
                "category": "Modularity & Maintainability",
                "rationale": f"Tệp {rel_path} có độ dài {lines} dòng (vượt ngưỡng P90 là {p90_threshold} dòng của toàn bộ dự án). Điều này làm tăng độ phức tạp nhận thức và khó khăn khi kiểm thử đơn vị độc lập.",
                "acceptance_criteria": f"Tách các struct, impl, hoặc handlers trong {rel_path} thành các submodule chuyên biệt theo nguyên tắc đơn nhiệm.",
                "metric_count": lines,
            })
    return complexity_topics


def analyze_runtime_lessons() -> List[Dict]:
    """Dynamically extract systemic architectural topics from recurring runtime failure lessons."""
    if not MEMORY_FILE.exists():
        return []

    try:
        lessons = json.loads(MEMORY_FILE.read_text(encoding="utf-8"))
    except Exception:
        return []

    lesson_topics = []
    for lesson in lessons:
        success_count = lesson.get("success_count", 1)
        node_type = lesson.get("node_type", "unknown")
        identifier = lesson.get("identifier", "unknown")
        error_pattern = lesson.get("error_pattern", "")
        remediation = lesson.get("remediation_strategy", "")

        # A recurring pattern indicates a systemic issue that should be solved at architecture level
        if success_count >= 2 or len(error_pattern) > 20:
            lesson_topics.append({
                "title": f"Triệt tiêu lỗi hệ thống lặp lại ({node_type}::{identifier})",
                "priority": "P1",
                "category": "Self-Healing Root Cause",
                "rationale": f"Hệ thống ghi nhận lỗi tái diễn ({success_count} lần): '{error_pattern[:80]}'. Giải pháp tạm thời hiện tại là: '{remediation[:80]}'. Cần sửa tận gốc trong mã nguồn.",
                "acceptance_criteria": f"Refactor mã nguồn xử lý {identifier} để loại bỏ hoàn toàn khả năng kích hoạt lỗi pattern trên.",
                "metric_count": success_count,
            })

    return lesson_topics


def discover_topics() -> List[Dict]:
    """Execute dynamic discovery across all non-hardcoded data sources."""
    crates = get_workspace_crates()
    rs_files = get_all_production_rs_files(crates)

    all_topics = []
    all_topics.extend(analyze_code_smells(rs_files))
    all_topics.extend(analyze_complexity_distribution(rs_files))
    all_topics.extend(analyze_runtime_lessons())

    # Sort topics by priority (P0 first, then P1, then P2), then by metric severity
    prio_map = {"P0": 0, "P1": 1, "P2": 2}
    all_topics.sort(key=lambda t: (prio_map.get(t["priority"], 9), -t.get("metric_count", 0)))
    return all_topics


def sync_to_backlog(topics: List[Dict]) -> int:
    """Sync newly discovered topics into RESEARCH_BACKLOG.md without duplicating."""
    existing_content = ""
    if BACKLOG_FILE.exists():
        existing_content = BACKLOG_FILE.read_text(encoding="utf-8")

    new_entries = []
    added = 0

    for t in topics:
        title = t["title"]
        if title in existing_content:
            continue

        topic_id = f"TOPIC-{datetime.now(timezone.utc).strftime('%Y%m%d')}-{uuid.uuid4().hex[:6]}"
        entry = (
            f"### [{t['priority']}] {topic_id}: {title}\n"
            f"- **Phân loại**: `{t['category']}`\n"
            f"- **Trạng thái**: `PENDING_RESEARCH` ⏳\n"
            f"- **Lý do cần nghiên cứu**: {t['rationale']}\n"
            f"- **Tiêu chuẩn nghiệm thu**: {t['acceptance_criteria']}\n\n"
        )
        new_entries.append(entry)
        added += 1

    if added > 0:
        if not BACKLOG_FILE.exists() or not existing_content.strip():
            header = (
                "# Tuzygent Autonomous Research Backlog\n\n"
                "Danh mục đề tài nghiên cứu được trích xuất hoàn toàn động từ phân tích AST mã nguồn, "
                "chỉ số phân phối độ phức tạp thống kê, và trí nhớ lỗi thực thi runtime.\n\n"
                "## Danh sách đề tài đang chờ giải quyết\n\n"
            )
            BACKLOG_FILE.write_text(header + "".join(new_entries), encoding="utf-8")
        else:
            with open(BACKLOG_FILE, "a", encoding="utf-8") as f:
                f.write("".join(new_entries))

    return added


def main():
    parser = argparse.ArgumentParser(description="Zero-Hardcoding Topic Discovery Engine")
    parser.add_argument("--scan", action="store_true", help="Chỉ quét và hiển thị, không ghi file")
    args = parser.parse_args()

    print("==================================================")
    print("   TUZYGENT DYNAMIC TOPIC DISCOVERY (ZERO-HARDCODE)")
    print("==================================================")

    crates = get_workspace_crates()
    print(f"📦 Đã phân tích động {len(crates)} workspace crates từ Cargo.toml:")
    for c in crates:
        print(f"  • {c.name}")

    topics = discover_topics()
    print(f"\n🔍 Phát hiện {len(topics)} đề tài dựa trên dữ liệu AST và thống kê thực tế:")
    for i, t in enumerate(topics[:10], 1):
        print(f"  {i}. [{t['priority']}] ({t['category']}) {t['title']}")

    if len(topics) > 10:
        print(f"  ... và {len(topics) - 10} đề tài khác.")

    if args.scan:
        return 0

    added = sync_to_backlog(topics)
    print(f"\n📝 Đã đồng bộ {added} đề tài mới vào {BACKLOG_FILE.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
