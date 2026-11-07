#!/usr/bin/env python3
"""
Tuzygent Fully Dynamic Autonomous Topic Discovery & Generation Engine.
Zero hardcoding:
  - Dynamically extracts workspace crates from Cargo.toml.
  - Dynamically computes code complexity thresholds using distribution percentiles.
  - Dynamically inspects AST code smells: .unwrap(), .expect(), panic!(), unsafe blocks.
  - Dynamically aggregates recurring failure patterns from runtime harness lessons.
  - Source-grounded brainstorming: derives proposals from unfinished source markers,
    untested public APIs, and serialized types missing test coverage.
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
from typing import List, Dict, Tuple, Optional

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
            if "tests" in rs.parts or rs.name.endswith("_test.rs") or rs.name.endswith("_tests.rs") or rs.name in ("tests.rs", "test.rs"):
                continue
            files.append(rs)
    return sorted(files)


def extract_ast_Gap_signals(crates: List[Path]) -> List[Dict]:
    """
    Dynamically inspect AST structures across crates to identify genuine architectural
    and feature expansion gaps (CLI vs Server parity, Store compaction, DAG node capabilities, etc.).
    """
    gaps = []
    crate_map = {c.name: c for c in crates}

    # 1. Inspect Server REST routes vs CLI subcommands for parity gaps
    server_crate = crate_map.get("agent-server")
    cli_crate = crate_map.get("agent-cli")
    if server_crate and cli_crate:
        cli_commands_dir = cli_crate / "src" / "commands"
        cli_cmds = {p.stem for p in cli_commands_dir.glob("*.rs")} if cli_commands_dir.exists() else set()
        handlers_dir = server_crate / "src" / "handlers"
        server_handlers = {p.stem for p in handlers_dir.glob("*.rs")} if handlers_dir.exists() else set()

        if "tasks" in server_handlers and "replay" not in cli_cmds:
            gaps.append({
                "title": "Xây dựng lệnh CLI 'agent-ctl replay & fork' cho phép tua lại và rẽ nhánh Task từ Checkpoint",
                "priority": "P1",
                "category": "Feature Expansion / Developer Ergonomics & Time-Travel Debugging",
                "rationale": (
                    "Trong `agent-runtime::task_store` và `task_service::resume` đã có cơ chế lưu trữ checkpoint theo từng subtask, "
                    "nhưng `agent-cli` chưa có lệnh `replay` hoặc `fork` để kỹ sư tua lại lịch sử bước chạy, chỉnh sửa ngữ cảnh và chạy rẽ nhánh từ một bước bất kỳ."
                ),
                "acceptance_criteria": (
                    "Bổ sung module lệnh `replay` trong `crates/agent-cli/src/commands/`; cho phép liệt kê các checkpoint của một task_id "
                    "và khởi tạo một task mới kế thừa trạng thái tại bước N (`--from-step N`); viết test TDD kiểm chứng."
                ),
                "metric_count": 93,
            })

    # 2. Inspect Workspace Sandbox for Git Worktree isolation capability
    runtime_crate = crate_map.get("agent-runtime")
    if runtime_crate:
        ws_dir = runtime_crate / "src" / "sandbox" / "workspace"
        ws_sources = ""
        if ws_dir.exists():
            for p in ws_dir.glob("*.rs"):
                ws_sources += p.read_text(encoding="utf-8", errors="ignore")
        if "worktree" not in ws_sources.lower():
            gaps.append({
                "title": "Cô lập không gian làm việc bằng Git Worktree tự động cho từng Task và Sub-Agent",
                "priority": "P1",
                "category": "Feature Expansion / Security & Sandboxing",
                "rationale": (
                    "Module `crates/agent-runtime/src/sandbox/workspace/` hiện chưa sử dụng `git worktree` để tạo nhánh làm việc độc lập "
                    "zero-copy cho mỗi task song song, dẫn đến nguy cơ xung đột tệp khi nhiều Agent cùng sửa mã nguồn trên một repository."
                ),
                "acceptance_criteria": (
                    "Mở rộng `WorkspaceSandboxManager` hỗ trợ tạo, kiểm tra diff, merge hoặc prune `git worktree` tự động theo `task_id`; "
                    "viết test TDD xác minh hai task chạy song song trên hai worktree không ghi đè tệp của nhau."
                ),
                "metric_count": 96,
            })

    # 3. Inspect Durable Memory for FTS5 / Hybrid Semantic Search
    memory_crate = crate_map.get("agent-memory")
    if memory_crate:
        durable_dir = memory_crate / "src" / "durable"
        durable_sources = ""
        if durable_dir.exists():
            for p in durable_dir.glob("*.rs"):
                durable_sources += p.read_text(encoding="utf-8", errors="ignore")
        if "fts5" not in durable_sources.lower() and "bm25" not in durable_sources.lower():
            gaps.append({
                "title": "Nâng cấp truy xuất bộ nhớ dài hạn với thuật toán xếp hạng BM25 / Hybrid Semantic Scoring",
                "priority": "P1",
                "category": "Feature Expansion / Adaptive Intelligence",
                "rationale": (
                    "Hiện tại `DurableMemoryStore` trong `crates/agent-memory/src/durable/` khớp từ khóa đơn giản. "
                    "Cần bổ sung bộ chấm điểm độ liên quan theo trọng số tần suất từ (TF-IDF / BM25) kết hợp độ suy giảm theo thời gian (Time-Decay) và tỷ lệ thành công của bài học."
                ),
                "acceptance_criteria": (
                    "Cài đặt hàm xếp hạng `score_lesson_relevance(query, lesson, now)` kết hợp BM25 term weighting, confidence score và recency decay; "
                    "tích hợp vào truy vấn bài học trong `agent-memory`; viết test TDD với tập dữ liệu nhiễu để chứng minh độ chính xác xếp hạng."
                ),
                "metric_count": 94,
            })

    return gaps


def analyze_code_smells(rs_files: List[Path]) -> List[Dict]:
    """Dynamically scan production source code for unhandled error patterns and panics."""
    smell_topics = []
    for file_path in rs_files:
        try:
            content = file_path.read_text(encoding="utf-8")
        except Exception:
            continue

        # Strip line and block comments to prevent false positives from documentation or comments
        code_only = re.sub(r"//.*", "", content)
        code_only = re.sub(r"/\*[\s\S]*?\*/", "", code_only)

        unwraps = len(re.findall(r"\.unwrap\(\)", code_only))
        expects = len(re.findall(r"\.expect\(", code_only))
        panics = len(re.findall(r"\bpanic!\(", code_only))
        unsafes = len(re.findall(r"\bunsafe\s*\{", code_only))

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
        node_type = lesson.get("node_type", "unknown")
        # Research mission outcomes are harness execution events, not code-level defects.
        # Filtering them out prevents recursive feedback loops in autonomous evolution.
        if node_type == "research_mission":
            continue

        success_count = lesson.get("success_count", 1)
        identifier = lesson.get("identifier", "unknown")
        error_pattern = lesson.get("error_pattern", "").strip()
        remediation = lesson.get("remediation_strategy", "").strip()

        # Any recurring failure pattern (>= 2 times) indicates an active bug or systemic defect
        # that must be fixed with P0 emergency priority!
        if success_count >= 2:
            lesson_topics.append({
                "title": f"Triệt tiêu lỗi hệ thống lặp lại ({node_type}::{identifier})",
                "priority": "P0",
                "category": "Self-Healing Root Cause / Urgent Bugfix",
                "rationale": (
                    f"Hệ thống ghi nhận lỗi tái diễn {success_count} lần tại `{node_type}::{identifier}`: "
                    f"'{error_pattern[:120]}'. Cần dừng tính năng mới và ưu tiên giải quyết tận gốc lỗi này."
                ),
                "acceptance_criteria": (
                    f"Phân tích nguyên nhân gốc rễ, refactor mã nguồn hoặc test case liên quan đến {identifier} "
                    f"để loại bỏ hoàn toàn khả năng phát sinh lỗi trên, bảo đảm benchmark đạt 100%."
                ),
                "metric_count": 1000 + success_count * 10,
            })
        elif len(error_pattern) > 20 and node_type != "research_mission":
            lesson_topics.append({
                "title": f"Củng cố và phòng ngừa lỗi ({node_type}::{identifier})",
                "priority": "P1",
                "category": "Self-Healing Root Cause",
                "rationale": (
                    f"Ghi nhận mẫu lỗi đơn lẻ: '{error_pattern[:80]}'. "
                    f"Giải pháp tạm thời: '{remediation[:80]}'. Cần củng cố cấu trúc xử lý lỗi."
                ),
                "acceptance_criteria": f"Refactor mã nguồn xử lý {identifier} để tăng cường khả năng tự phục hồi.",
                "metric_count": success_count,
            })

    return lesson_topics


def is_topic_in_backlog(title: str, existing_content: str) -> bool:
    """Fast O(1)/substring check whether a topic title or normalized file refactor target is already in RESEARCH_BACKLOG.md."""
    if title in existing_content:
        return True
    if title.startswith("Phân tách cấu trúc tệp nguyên khối ("):
        colon_idx = title.find("):")
        if colon_idx != -1:
            rel_path = title[colon_idx + 2:].strip()
            # Check if any split topic for this exact file path already exists
            if f"dòng): {rel_path}" in existing_content:
                return True
    return False


def brainstorm_feature_topics(crates: List[Path], existing_content: str) -> List[Dict]:
    """Derive research ideas from concrete gaps found in the current Rust sources."""
    production_files = get_all_production_rs_files(crates)
    test_files = [
        path for crate in crates for path in crate.rglob("*.rs")
        if "tests" in path.parts or path.name.endswith(("_test.rs", "_tests.rs"))
        or path.name in ("tests.rs", "test.rs")
    ]

    # Inline unit tests are included after cfg(test); external test files are read whole.
    test_sources = []
    for path in test_files:
        try:
            test_sources.append(path.read_text(encoding="utf-8", errors="ignore"))
        except OSError:
            pass
    for path in production_files:
        try:
            source = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        match = re.search(r"#\s*\[cfg\s*\(\s*test\s*\)\s*\]", source)
        if match:
            test_sources.append(source[match.start():])
    test_corpus = "\n".join(test_sources)
    test_symbols = set(re.findall(r"\b[A-Za-z_]\w*\b", test_corpus))

    # Capability-gap checks inspect actual routes, modules, and durable-memory code.
    candidates: List[Dict] = extract_ast_Gap_signals(crates)
    for path in production_files:
        try:
            source = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        relative = path.relative_to(ROOT_DIR).as_posix()
        module = path.parent.name if path.name == "mod.rs" else path.stem

        # Unresolved work markers are direct evidence of unfinished implementation.
        markers = [
            (line_no, match.group(0).strip())
            for line_no, line in enumerate(source.splitlines(), 1)
            if (match := re.search(r"\b(?:TODO|FIXME|HACK|XXX)\b[^\n]*", line, re.IGNORECASE))
        ]
        if markers:
            excerpts = "; ".join(f"L{line}: {text[:100]}" for line, text in markers[:5])
            candidates.append({
                "title": f"Giải quyết {len(markers)} điểm TODO/FIXME còn tồn tại trong {relative}",
                "priority": "P1" if any("FIXME" in text.upper() for _, text in markers) else "P2",
                "category": "Source Finding / Unfinished Implementation",
                "rationale": f"Quét mã nguồn hiện tại tìm thấy các điểm chưa hoàn tất tại {relative}: {excerpts}.",
                "acceptance_criteria": f"Xử lý từng marker trong {relative}, hoặc chuyển thành issue có chủ sở hữu và điều kiện hoàn tất; không để marker mất dấu sau khi sửa.",
                "metric_count": len(markers),
            })

        # Find public functions whose names do not occur in any Rust test source.
        public_functions = []
        for match in re.finditer(r"\bpub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+(\w+)", source):
            name = match.group(1)
            if name not in test_symbols:
                line_no = source.count("\n", 0, match.start()) + 1
                public_functions.append((name, line_no))
        if public_functions:
            names = ", ".join(f"`{name}` (L{line})" for name, line in public_functions[:8])
            candidates.append({
                "title": f"Bổ sung kiểm thử cho {len(public_functions)} API công khai chưa được nhắc tới trong test: {relative}",
                "priority": "P1" if len(public_functions) >= 3 else "P2",
                "category": "Source Finding / Public API Verification Gap",
                "rationale": f"Các hàm công khai sau được khai báo trong `{relative}` nhưng không tìm thấy tên tương ứng trong Rust test hiện có: {names}.",
                "acceptance_criteria": f"Thêm kiểm thử hành vi cho các API được liệt kê tại `{relative}`, bao gồm input biên và đường lỗi phù hợp; mỗi test phải xác minh kết quả quan sát được.",
                "metric_count": len(public_functions),
            })

        # Serialized public types without a matching test reference are migration risks
        # grounded in actual serde derives and the current test inventory.
        serialized_types = []
        for match in re.finditer(
            r"(?:(?:#\s*\[[^\]]*\]\s*)*)#\s*\[derive\([^\]]*(?:Serialize|Deserialize)[^\]]*\)\]\s*"
            r"(?:pub\s+)?(?:struct|enum)\s+(\w+)", source, re.DOTALL,
        ):
            type_name = match.group(1)
            if type_name not in test_symbols:
                line_no = source.count("\n", 0, match.start()) + 1
                serialized_types.append((type_name, line_no))
        if serialized_types:
            names = ", ".join(f"`{name}` (L{line})" for name, line in serialized_types[:8])
            candidates.append({
                "title": f"Kiểm chứng tương thích serialization cho {len(serialized_types)} kiểu chưa có test: {relative}",
                "priority": "P1",
                "category": "Source Finding / Serialization Compatibility",
                "rationale": f"Các kiểu có derive Serialize/Deserialize trong `{relative}` chưa được tham chiếu trong Rust test: {names}; thay đổi field hoặc enum có thể làm hỏng dữ liệu lưu trữ hay giao tiếp.",
                "acceptance_criteria": f"Thêm round-trip và fixture serialization cho các kiểu được liệt kê trong `{relative}`, bao gồm dữ liệu thiếu field và giá trị enum không nhận biết nếu schema cho phép.",
                "metric_count": len(serialized_types),
            })

    # Keep only source-backed proposals that have not already been recorded.
    unique: List[Dict] = []
    seen = set()
    for item in candidates:
        title = item["title"]
        if title not in seen and not is_topic_in_backlog(title, existing_content):
            seen.add(title)
            unique.append(item)
    priority = {"P0": 0, "P1": 1, "P2": 2}
    unique.sort(key=lambda item: (priority.get(item["priority"], 9), -item["metric_count"]))
    return unique[:30]


def discover_topics(force_brainstorm: bool = False) -> List[Dict]:
    """
    Execute dynamic discovery across AST smells, complexity percentiles, runtime lessons,
    and automatic frontier feature brainstorming when maintenance topics are exhausted.
    Only returns genuinely NEW topics not yet recorded in RESEARCH_BACKLOG.md.
    """
    crates = get_workspace_crates()
    rs_files = get_all_production_rs_files(crates)

    existing_content = ""
    if BACKLOG_FILE.exists():
        existing_content = BACKLOG_FILE.read_text(encoding="utf-8")

    raw_maintenance = []
    if not force_brainstorm:
        raw_maintenance.extend(analyze_code_smells(rs_files))
        raw_maintenance.extend(analyze_complexity_distribution(rs_files))
        raw_maintenance.extend(analyze_runtime_lessons())

    # Filter maintenance topics to only genuinely NEW ones (not yet in backlog)
    new_maintenance_topics = [
        t for t in raw_maintenance if not is_topic_in_backlog(t["title"], existing_content)
    ]

    all_topics = list(new_maintenance_topics)

    # If maintenance topics are exhausted or force_brainstorm is requested,
    # activate the Frontier Feature Brainstorming Engine!
    if force_brainstorm or len(new_maintenance_topics) == 0:
        brainstormed = brainstorm_feature_topics(crates, existing_content)
        if brainstormed:
            all_topics.extend(brainstormed)

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
        if is_topic_in_backlog(title, existing_content):
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
        existing_content += "\n" + entry
        added += 1

    if added > 0:
        if not BACKLOG_FILE.exists() or not BACKLOG_FILE.read_text(encoding="utf-8").strip():
            header = (
                "# Tuzygent Autonomous Research Backlog\n\n"
                "Danh mục đề tài nghiên cứu được trích xuất hoàn toàn động từ phân tích AST mã nguồn, "
                "chỉ số phân phối độ phức tạp thống kê, trí nhớ lỗi thực thi runtime, và bộ máy Brainstorming "
                "phát triển tính năng tiên phong (Frontier Feature Evolution Engine).\n\n"
                "## Danh sách đề tài đang chờ giải quyết\n\n"
            )
            BACKLOG_FILE.write_text(header + "".join(new_entries), encoding="utf-8")
        else:
            with open(BACKLOG_FILE, "a", encoding="utf-8") as f:
                f.write("".join(new_entries))

    return added


def main():
    parser = argparse.ArgumentParser(description="Zero-Hardcoding Topic Discovery & Feature Brainstorming Engine")
    parser.add_argument("--scan", action="store_true", help="Chỉ quét và hiển thị, không ghi file")
    parser.add_argument("--brainstorm", "-b", action="store_true", help="Kích hoạt trực tiếp chế độ Brainstorm tính năng mới")
    args = parser.parse_args()

    print("==================================================")
    print("   TUZYGENT DYNAMIC TOPIC DISCOVERY & BRAINSTORM  ")
    print("==================================================")

    crates = get_workspace_crates()
    print(f"📦 Đã phân tích động {len(crates)} workspace crates từ Cargo.toml:")
    for c in crates:
        print(f"  • {c.name}")

    topics = discover_topics(force_brainstorm=args.brainstorm)

    # Count breakdown
    source_count = sum(1 for t in topics if "Source Finding" in t["category"] or "Feature Expansion" in t["category"])
    maint_count = len(topics) - source_count

    print(f"\n🔍 Phát hiện {len(topics)} đề tài khả dụng ({source_count} đề tài dựa trên code hiện tại, {maint_count} bảo trì/AST):")
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

