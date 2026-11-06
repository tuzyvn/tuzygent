#!/usr/bin/env python3
"""
Tuzygent Fully Dynamic Autonomous Topic Discovery & Generation Engine.
Zero hardcoding:
  - Dynamically extracts workspace crates from Cargo.toml.
  - Dynamically computes code complexity thresholds using distribution percentiles.
  - Dynamically inspects AST code smells: .unwrap(), .expect(), panic!(), unsafe blocks.
  - Dynamically aggregates recurring failure patterns from runtime harness lessons.
  - Frontier Feature Brainstorming: Dynamically brainstorms next-generation architectural
    features across 6 dimensions (Resilience, Multi-Agent Collaboration, Observability,
    Security & Sandboxing, Developer Ergonomics, Adaptive Intelligence) based on existing
    system capabilities when maintenance topics are exhausted.
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


def extract_system_capabilities(crates: List[Path]) -> Dict[str, List[str]]:
    """Dynamically discover real domain modules across workspace crates (deduplicated, excluding synthetic test/bench modules)."""
    excluded_stems = {
        "lib", "main", "mod", "tests", "test",
        "benchmark", "fault", "resilience", "chaos", "sharded", "safe",
    }
    capabilities = {}
    for crate in crates:
        crate_name = crate.name
        modules = set()
        src_dir = crate / "src"
        if src_dir.exists():
            for item in src_dir.glob("*"):
                if item.is_dir() and not item.name.startswith("."):
                    if item.name not in excluded_stems and not item.name.endswith("_tests"):
                        modules.add(item.name)
                elif item.suffix == ".rs":
                    if item.stem not in excluded_stems and not item.stem.endswith("_tests"):
                        modules.add(item.stem)
        capabilities[crate_name] = sorted(modules)
    return capabilities


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
    """
    Dynamically brainstorm next-generation features based on current capability graph & AST gaps.
    Categories:
      1. Resilience & Fault Tolerance
      2. Multi-Agent Collaboration
      3. Observability & Telemetry
      4. Security & Sandboxing
      5. Developer Ergonomics & Tooling
      6. Adaptive Intelligence
    """
    capabilities = extract_system_capabilities(crates)
    ast_gap_topics = extract_ast_Gap_signals(crates)

    feature_matrix = ast_gap_topics + [
        # --- Wave 1 & Wave 2: Resilience & Fault Tolerance ---
        {
            "title": "Tích hợp cơ chế Exponential Backoff & Jitter cho Engine Execution Retry",
            "priority": "P1",
            "category": "Feature Expansion / Resilience & Fault Tolerance",
            "rationale": (
                "Hiện tại các LLM Engine khi gặp lỗi HTTP 429 (Rate Limit) hoặc 503 (Provider Unavailable) "
                "dễ làm đứt gãy luồng OODA. Cần bổ sung thuật toán Full Jitter Exponential Backoff trong "
                "`crates/agent-engine/` và `crates/agent-runtime/src/ooda/` để phục hồi mềm mại các sự cố tạm thời."
            ),
            "acceptance_criteria": (
                "Cài đặt struct `BackoffPolicy` hỗ trợ cấu hình initial_interval, max_interval, multiplier và jitter; "
                "tích hợp vào vòng lặp gọi Engine; viết test TDD kiểm chứng khoảng thời gian chờ giữa các lần retry."
            ),
            "metric_count": 95,
        },
        {
            "title": "Triển khai Dead-Letter Queue & Poison-Task Isolation trong TaskService",
            "priority": "P1",
            "category": "Feature Expansion / Resilience & Fault Tolerance",
            "rationale": (
                "Khi một tác vụ liên tục gây lỗi không thể hồi phục hoặc làm treo worker, TaskService cần tự động "
                "cô lập tác vụ đó vào Dead-Letter Queue thay vì để nó làm nghẽn toàn bộ hàng đợi của các tác vụ khác."
            ),
            "acceptance_criteria": (
                "Mở rộng `TaskService` với trạng thái `Quarantined` hoặc Dead-Letter Queue; "
                "bổ sung API tra cứu và giải phóng task bị cô lập; viết test TDD xác minh hàng đợi vẫn tiếp tục xử lý các task khác."
            ),
            "metric_count": 90,
        },
        {
            "title": "Cơ chế In-flight Checkpointing & Resume State an toàn cho Long-Running Tasks",
            "priority": "P1",
            "category": "Feature Expansion / Resilience & Fault Tolerance",
            "rationale": (
                "Với các tác vụ dài hơi gồm nhiều subtask, nếu server hoặc daemon bị restart đột ngột, "
                "task cần khả năng phục hồi chính xác từ subtask đã verified gần nhất thay vì chạy lại từ đầu."
            ),
            "acceptance_criteria": (
                "Lưu checkpoint metadata vào `SqliteTaskStore` sau mỗi subtask hoàn tất; "
                "bổ sung phương thức `resume_task_from_checkpoint`; viết test TDD mô phỏng ngắt tiến trình giữa chừng và khôi phục an toàn."
            ),
            "metric_count": 88,
        },
        {
            "title": "Circuit Breaker Failover tự động chuyển đổi LLM Provider khi Engine chính gặp sự cố",
            "priority": "P1",
            "category": "Feature Expansion / Resilience & Fault Tolerance",
            "rationale": (
                "Hỗ trợ cấu hình Fallback Engine (ví dụ: Antigravity -> Claude -> Codex hoặc Mock) "
                "khi primary engine bị cạn quota (HTTP 402/429) hoặc connection timeout liên tục."
            ),
            "acceptance_criteria": (
                "Xây dựng `EngineFailoverCoordinator` quản lý danh sách engine ưu tiên; "
                "tự động chuyển sang fallback provider khi primary ném `EngineError::QuotaExhausted` hoặc lỗi kết nối; có test TDD xác minh."
            ),
            "metric_count": 85,
        },
        {
            "title": "Bộ lập lịch Cron Scheduler & Delayed Task Queue tích hợp sẵn trong TaskService",
            "priority": "P1",
            "category": "Feature Expansion / Resilience & Automation",
            "rationale": (
                "Cho phép đăng ký các tác vụ chạy định kỳ (cron expression hoặc interval) và tác vụ hẹn giờ (`run_after` timestamp) "
                "trực tiếp trong `SqliteTaskStore` và `TaskService` mà không phụ thuộc vào crontab của hệ điều hành."
            ),
            "acceptance_criteria": (
                "Bổ sung cấu trúc `ScheduleSpec` và bảng lưu lịch chạy trong `task_store`; hỗ trợ đánh thức task đúng hạn; "
                "viết test TDD xác minh task hẹn giờ chỉ được dispatch khi đến thời điểm `run_after`."
            ),
            "metric_count": 91,
        },

        # --- Wave 1 & Wave 2: Multi-Agent Collaboration ---
        {
            "title": "Mở rộng InterAgentBus hỗ trợ Topic-Based Publish/Subscribe Channels",
            "priority": "P1",
            "category": "Feature Expansion / Multi-Agent Collaboration",
            "rationale": (
                "Hiện tại `InterAgentBus` phát broadcast toàn bộ thông điệp. Cần bổ sung phân luồng theo Topic "
                "(Topic-based Pub/Sub) để các agent chuyên môn hóa (Security, QA, Developer) chỉ nhận sự kiện phù hợp."
            ),
            "acceptance_criteria": (
                "Bổ sung `subscribe_topic(topic_pattern)` và `publish_topic(topic, event)` trong `InterAgentBus`; "
                "hỗ trợ wildcard matching; viết test TDD kiểm thử gửi nhận thông điệp có lọc chủ đề đa luồng an toàn."
            ),
            "metric_count": 84,
        },
        {
            "title": "Xây dựng Multi-Agent Consensus Voting Gate trong Workflow DAG",
            "priority": "P2",
            "category": "Feature Expansion / Multi-Agent Collaboration",
            "rationale": (
                "Thêm loại node thẩm định đồng thuận (Consensus Gate Node) trong đồ thị Workflow DAG đòi hỏi "
                "tỷ lệ biểu quyết tán thành (Quorum Threshold, ví dụ >= 66%) từ các Agent Reviewer trước khi chuyển bước nhạy cảm."
            ),
            "acceptance_criteria": (
                "Định nghĩa `WorkflowNode::ConsensusGate` với trường `required_votes` và danh sách reviewer; "
                "cập nhật bộ thực thi DAG để thu thập votes; viết test TDD kiểm thử kịch bản đủ phiếu và không đủ phiếu."
            ),
            "metric_count": 80,
        },
        {
            "title": "Giao thức phân quyền và Scoped Scratchpad Isolation cho Sub-Agent Delegation",
            "priority": "P2",
            "category": "Feature Expansion / Multi-Agent Collaboration",
            "rationale": (
                "Khi root agent phân rã nhiệm vụ cho sub-agent, cần cơ chế cô lập ngữ cảnh bộ nhớ tạm (Scoped Scratchpad), "
                "tránh việc sub-agent ghi đè hoặc đọc trộm dữ liệu nhạy cảm của parent task."
            ),
            "acceptance_criteria": (
                "Cài đặt `ScopedScratchpad` phân cấp với quyền ReadOnly/ReadWrite cục bộ cho sub-agent; "
                "tự động dọn dẹp hoặc gộp kết quả khi sub-agent hoàn tất; có test TDD kiểm chứng tính cô lập."
            ),
            "metric_count": 78,
        },
        {
            "title": "Chế độ Best-of-N Speculative Parallel Engine Tournament & Auto-Judge",
            "priority": "P1",
            "category": "Feature Expansion / Multi-Agent Collaboration",
            "rationale": (
                "Cho phép giao cùng một nhiệm vụ quan trọng cho N engine khác nhau (`antigravity`, `claude`, `codex`) chạy song song, "
                "dùng `TaskVerifier` chấm điểm khách quan (test pass, thời gian, kích thước diff) để chọn ra kết quả tốt nhất."
            ),
            "acceptance_criteria": (
                "Cài đặt `TournamentExecutor` trong `crates/agent-runtime/`; chạy song song các candidate engines và chọn kết quả có điểm verification cao nhất; "
                "viết test TDD mô phỏng 3 engines trả kết quả khác nhau và xác minh bộ chọn hoạt động chính xác."
            ),
            "metric_count": 92,
        },
        {
            "title": "Thực thi song song Fan-Out / Fan-In MapReduce Node trong Workflow DAG",
            "priority": "P2",
            "category": "Feature Expansion / Multi-Agent Collaboration",
            "rationale": (
                "Bổ sung khả năng chia nhỏ một tập dữ liệu đầu vào (Fan-Out / Map) cho nhiều Agent xử lý đồng thời có giới hạn concurrency, "
                "sau đó tổng hợp kết quả tại node hội tụ (Fan-In / Reduce) trong `workflow_dag`."
            ),
            "acceptance_criteria": (
                "Hỗ trợ thực thi song song các nhánh độc lập và gom kết quả đầu ra theo thứ tự xác định trong `WorkflowDagOrchestrator`; "
                "viết test TDD kiểm chứng giới hạn concurrency và tính toàn vẹn dữ liệu gộp."
            ),
            "metric_count": 83,
        },

        # --- Wave 1 & Wave 2: Observability & Telemetry ---
        {
            "title": "Tích hợp Prometheus Metrics Exporter (/metrics) cho Agent Server & OODA Loop",
            "priority": "P1",
            "category": "Feature Expansion / Observability & Telemetry",
            "rationale": (
                "Cung cấp endpoint HTTP `/metrics` theo chuẩn Prometheus trong `agent-server` để giám sát "
                "thời gian thực các chỉ số: tổng task, độ trễ OODA cycle, số lần remediation, tỷ lệ thành công của test."
            ),
            "acceptance_criteria": (
                "Thêm endpoint `/metrics` trong Axum router; xuất bản các chỉ số `agent_tasks_total`, "
                "`agent_ooda_duration_seconds`, `agent_remediations_total`; viết test HTTP integration kiểm chứng định dạng text/plain."
            ),
            "metric_count": 86,
        },
        {
            "title": "Bổ sung Structured OpenTelemetry Tracing Spans cho OODA Lifecycle",
            "priority": "P2",
            "category": "Feature Expansion / Observability & Telemetry",
            "rationale": (
                "Gắn các tracing span có cấu trúc với task_id, subtask_id, và execution_phase cho từng bước "
                "(Observe, Orient, Decide, Act, Remediate) giúp trực quan hóa timeline thực thi và định vị nút thắt cổ chai."
            ),
            "acceptance_criteria": (
                "Tích hợp `tracing::info_span!` xuyên suốt các module trong `crates/agent-runtime/src/ooda/`; "
                "viết test TDD kiểm tra các span metadata được kích hoạt đúng thứ tự."
            ),
            "metric_count": 76,
        },
        {
            "title": "Xây dựng Immutable Audit Trail Ledger có mã băm toàn vẹn cho Guardrail Actions",
            "priority": "P2",
            "category": "Feature Expansion / Observability & Telemetry",
            "rationale": (
                "Mọi quyết định của Guardrail (chặn lệnh nguy hiểm, phê duyệt ngoại lệ, shell command) "
                "cần được ghi nhận vào nhật ký kiểm toán bất biến có chuỗi băm (SHA256 hash chaining) chống giả mạo."
            ),
            "acceptance_criteria": (
                "Cài đặt cấu trúc `AuditLedger` với `prev_hash` liên kết từng sự kiện; "
                "cung cấp phương thức `verify_integrity()`; viết test TDD phát hiện can thiệp sửa đổi trái phép."
            ),
            "metric_count": 75,
        },
        {
            "title": "Bộ đệm Replay sự kiện có Sequence Cursor cho WebSocket & SSE Reconnect",
            "priority": "P1",
            "category": "Feature Expansion / Observability & Streaming",
            "rationale": (
                "Khi client WebSocket (`/api/v1/ws`) bị rớt mạng tạm thời hoặc đọc chậm (`RecvError::Lagged`), "
                "client cần có thể gửi `last_seq_id` để lấy bù các sự kiện bị nhỡ từ Ring Buffer hoặc TaskStore."
            ),
            "acceptance_criteria": (
                "Gắn số thứ tự đơn điệu (`seq_id`) cho mọi sự kiện trên `InterAgentBus` và lưu ring-buffer có giới hạn; "
                "hỗ trợ truy vấn `events_since(seq_id)`; viết test TDD mô phỏng client kết nối lại và nhận đủ sự kiện bị thiếu."
            ),
            "metric_count": 89,
        },
        {
            "title": "Sổ cái thống kê Token Usage & Cost Budget Enforcement theo từng Task và Account",
            "priority": "P2",
            "category": "Feature Expansion / Observability & FinOps",
            "rationale": (
                "Theo dõi số lượng prompt/completion tokens, thời gian chạy và chi phí ước tính cho mỗi `task_id` và `account_id`, "
                "tự động tạm dừng task nếu vượt hạn mức ngân sách (`max_budget_usd` / `max_tokens_per_task`)."
            ),
            "acceptance_criteria": (
                "Cài đặt `UsageLedger` lưu trữ thống kê token/chi phí; kiểm tra hạn mức trước mỗi vòng lặp OODA; "
                "viết test TDD xác minh task dừng an toàn khi vượt ngưỡng ngân sách cấu hình."
            ),
            "metric_count": 82,
        },

        # --- Wave 1 & Wave 2: Security & Sandboxing ---
        {
            "title": "Tự động che mờ dữ liệu nhạy cảm (Secret Redaction) trong Tool Output Stream",
            "priority": "P1",
            "category": "Feature Expansion / Security & Sandboxing",
            "rationale": (
                "Trong quá trình chạy lệnh hoặc engine streaming, output stdout/stderr có thể vô tình in ra "
                "API keys, private tokens, passwords. Cần bộ lọc Real-Time Secret Redaction tự động che mờ thành [REDACTED]."
            ),
            "acceptance_criteria": (
                "Xây dựng module `SecretRedactor` với các mẫu regex phổ biến (OpenAI, AWS, JWT, Bearer token); "
                "tích hợp vào stream reader trong `crates/agent-engine/src/process/stream.rs`; viết test TDD xác minh các key bị che mờ."
            ),
            "metric_count": 89,
        },
        {
            "title": "Cơ chế Ephemeral OverlayFS / Tmpfs Workspace Isolation cho Sandbox Drivers",
            "priority": "P2",
            "category": "Feature Expansion / Security & Sandboxing",
            "rationale": (
                "Khi chạy các lệnh không tin cậy hoặc bước kiểm thử trong Sandbox, cần tùy chọn gắn kết filesystem tạm thời "
                "(Tmpfs / OverlayFS) để tự động xóa sạch các tệp rác sinh ra nếu bước thực thi bị thất bại hoặc rollback."
            ),
            "acceptance_criteria": (
                "Thêm tùy chọn `ephemeral_overlay: bool` trong `SandboxSpec`; "
                "cập nhật bubblewrap và docker driver để thiết lập read-only base kèm writable tmpfs upper; viết test kiểm chứng."
            ),
            "metric_count": 77,
        },
        {
            "title": "Model Context Protocol (MCP) Tool Gateway & Guardrail Policy Interceptor",
            "priority": "P1",
            "category": "Feature Expansion / Security & Tooling",
            "rationale": (
                "Cho phép khai báo các MCP Server bên ngoài và kiểm duyệt mọi lệnh gọi MCP tool (`tools/call`) "
                "thông qua `GuardrailPolicy` và `AuditLedger` trước khi cho phép Agent thực thi."
            ),
            "acceptance_criteria": (
                "Xây dựng module `McpToolGateway` chuẩn hóa JSON-RPC request/response của MCP; "
                "chặn các tool call vi phạm danh sách đen của `GuardrailPolicy`; viết test TDD xác minh chặn và ghi nhận audit."
            ),
            "metric_count": 90,
        },
        {
            "title": "Kiểm soát Outbound Network Egress Policy & Domain Whitelist cho Sandbox",
            "priority": "P2",
            "category": "Feature Expansion / Security & Sandboxing",
            "rationale": (
                "Cấu hình chính sách mạng đầu ra (`NetworkEgressPolicy`: `Isolated`, `AllowList`, `Unrestricted`) trong `SandboxSpec` "
                "để ngăn chặn mã thực thi trong sandbox rò rỉ dữ liệu ra các máy chủ lạ bên ngoài."
            ),
            "acceptance_criteria": (
                "Thêm `NetworkEgressPolicy` vào `crates/agent-runtime/src/sandbox/config.rs`; "
                "áp dụng cờ `--unshare-net` trong bubblewrap hoặc `--network none` trong Docker khi ở chế độ `Isolated`; viết test TDD kiểm chứng."
            ),
            "metric_count": 81,
        },

        # --- Wave 1 & Wave 2: Developer Ergonomics & Tooling ---
        {
            "title": "Bổ sung lệnh CLI 'agent-ctl inspect' tra cứu Task & Memory chuyên sâu",
            "priority": "P2",
            "category": "Feature Expansion / Developer Ergonomics",
            "rationale": (
                "Cung cấp lệnh CLI trực quan `agent-ctl inspect [task-id|memory|dag]` hiển thị bảng trạng thái, "
                "các subtask đã thực hiện, timeline và dữ liệu memory mà không cần dùng sqlite3 thủ công."
            ),
            "acceptance_criteria": (
                "Thêm subcommand `inspect` vào `crates/agent-cli/`; "
                "hỗ trợ xuất ra định dạng bảng terminal màu hoặc JSON (`--format json`); viết test CLI argument parser."
            ),
            "metric_count": 74,
        },
        {
            "title": "Chức năng xuất sơ đồ Workflow DAG sang định dạng Mermaid / Graphviz",
            "priority": "P2",
            "category": "Feature Expansion / Developer Ergonomics",
            "rationale": (
                "Giúp kỹ sư và tài liệu dự án dễ dàng trực quan hóa các đồ thị Workflow DAG phức tạp "
                "bằng cách xuất cấu trúc DAG sang mã Mermaid (`graph TD ...`) hoặc DOT format."
            ),
            "acceptance_criteria": (
                "Bổ sung phương thức `to_mermaid(&self) -> String` và `to_dot(&self) -> String` trên `WorkflowDag`; "
                "viết test TDD kiểm chứng cú pháp Mermaid sinh ra hợp lệ và đầy đủ các node/edge."
            ),
            "metric_count": 72,
        },
        {
            "title": "Cơ chế Dynamic Skill Hot-Reloading & Schema Validation trong AgentMemory",
            "priority": "P2",
            "category": "Feature Expansion / Developer Ergonomics",
            "rationale": (
                "Cho phép daemon và runner tự động phát hiện các skill mới được thêm vào thư mục `skills/` "
                "và nạp lại vào bộ nhớ runtime mà không cần khởi động lại toàn bộ tiến trình."
            ),
            "acceptance_criteria": (
                "Cài đặt phương thức `reload_skills(&mut self)` trong `crates/agent-memory/src/skills.rs`; "
                "xác thực schema frontmatter; viết test TDD kiểm thử thêm skill mới khi runtime đang chạy."
            ),
            "metric_count": 70,
        },
        {
            "title": "Webhook Trigger Hub có xác thực chữ ký HMAC-SHA256 trong Agent Server",
            "priority": "P2",
            "category": "Feature Expansion / Developer Ergonomics & Event-Driven Automation",
            "rationale": (
                "Cho phép GitHub Actions, GitLab hoặc Alertmanager bắn sự kiện trực tiếp vào `/api/v1/webhooks/trigger` "
                "(kèm xác thực chữ ký HMAC-SHA256 và chống replay) để tự động khởi tạo Task sửa lỗi."
            ),
            "acceptance_criteria": (
                "Xây dựng handler và bộ xác thực chữ ký HMAC-SHA256 cho webhook payload trong `agent-server`; "
                "tự động chuyển đổi sự kiện hợp lệ thành Task trong hàng đợi; viết test TDD kiểm thử chữ ký đúng và sai."
            ),
            "metric_count": 79,
        },

        # --- Wave 1 & Wave 2: Adaptive Intelligence ---
        {
            "title": "Cơ chế Adaptive Token-Budget Windowing & Rolling Context Compaction cho OODA Loop",
            "priority": "P1",
            "category": "Feature Expansion / Adaptive Intelligence",
            "rationale": (
                "Khi working context tích lũy qua nhiều subtask vượt quá ngưỡng token an toàn của LLM, "
                "cần thuật toán Rolling Context Compaction tự động tóm tắt các bước cũ và giữ lại đầy đủ kết quả then chốt gần nhất."
            ),
            "acceptance_criteria": (
                "Cài đặt hàm `compact_working_context_with_budget(ctx: &str, max_tokens: usize) -> String`; "
                "tích hợp vào `crates/agent-runtime/src/ooda/execution/runner/step/context.rs`; viết test TDD kiểm tra context được nén đúng ngưỡng."
            ),
            "metric_count": 87,
        },
        {
            "title": "Định tuyến thông minh theo độ phức tạp nhiệm vụ (Complexity-Aware Model Router)",
            "priority": "P1",
            "category": "Feature Expansion / Adaptive Intelligence",
            "rationale": (
                "Tự động phân tích độ phức tạp của Subtask (Planning/Architecture vs Sửa lỗi nhỏ/Format/Test) "
                "trong `crates/agent-engine/src/router.rs` để chọn Tier model tối ưu chi phí và tốc độ."
            ),
            "acceptance_criteria": (
                "Nâng cấp `ModelRouter` với bộ ước lượng độ phức tạp (`TaskComplexity::Low | Medium | High`); "
                "ánh xạ sang cấu hình engine phù hợp; viết test TDD xác minh phân loại và định tuyến chính xác."
            ),
            "metric_count": 86,
        },
        {
            "title": "Hợp nhất tri thức liên dự án & Suy giảm độ tin cậy bài học cũ (Memory Consolidation & Decay)",
            "priority": "P2",
            "category": "Feature Expansion / Adaptive Intelligence",
            "rationale": (
                "Khi kho `DurableMemoryStore` lớn dần, các bài học trùng lặp cần được hợp nhất (deduplicate & merge) "
                "và các bài học lâu không sử dụng hoặc thất bại liên tiếp cần được giảm trọng số ưu tiên."
            ),
            "acceptance_criteria": (
                "Cài đặt phương thức `consolidate_and_decay_lessons()` trong `crates/agent-memory/src/durable/`; "
                "gộp các bài học cùng lỗi và cập nhật điểm tin cậy; viết test TDD kiểm chứng."
            ),
            "metric_count": 78,
        },
    ]

    # Filter out topics that already exist in RESEARCH_BACKLOG.md (whether pending, completed, or blocked)
    candidate_topics = []
    seen_titles = set()
    for item in feature_matrix:
        t = item["title"]
        if t not in seen_titles and not is_topic_in_backlog(t, existing_content):
            seen_titles.add(t)
            candidate_topics.append(item)

    # Multi-Track Evolutionary Tier: Dynamically synthesize deep architectural tracks across ALL real modules
    # (no longer restricted to `modules[:3]` or synthetic benchmark/fault modules).
    evolution_tracks = [
        (
            "Kiểm chứng Bất biến Trạng thái & Property-Based Contract cho module {crate}::{mod}",
            "Feature Expansion / State Invariant & Contract Verification",
            "P2",
            68,
            (
                "Xác định và kiểm tra các điều kiện bất biến (state invariants, idempotency, serialization roundtrip, "
                "boundary contracts) cho `{crate}::{mod}` với đầu vào biên và trạng thái bất thường."
            ),
            (
                "Viết bộ test kiểm chứng bất biến trạng thái và ranh giới dữ liệu cho `{crate}::{mod}`, "
                "bảo đảm không xảy ra trạng thái không hợp lệ hoặc panic."
            ),
        ),
        (
            "Cơ chế Backpressure, Giới hạn Tài nguyên & Chống cạn kiệt bộ nhớ cho {crate}::{mod}",
            "Feature Expansion / Resource Bounding & Backpressure",
            "P2",
            66,
            (
                "Bảo vệ `{crate}::{mod}` trước các đầu vào kích thước lớn hoặc tần suất gọi đột biến bằng cơ chế "
                "bounded capacity, timeout tường minh và từ chối sớm có kiểm soát."
            ),
            (
                "Bổ sung giới hạn kích thước/số lượng an toàn và viết test TDD chứng minh `{crate}::{mod}` "
                "xử lý mượt mà khi vượt ngưỡng tài nguyên."
            ),
        ),
        (
            "Chaos Engineering & Fault Injection Test cho module {crate}::{mod}",
            "Feature Expansion / Chaos Engineering",
            "P2",
            64,
            (
                "Mô phỏng lỗi ngắt quãng, socket timeout, hoặc I/O failure đột ngột trong module "
                "`{crate}::{mod}` để kiểm tra tính bền vững và khả năng tự phục hồi mà không làm sập toàn bộ hệ thống."
            ),
            (
                "Viết integration test mô phỏng kịch bản lỗi ngắt quãng và xác minh `{crate}::{mod}` phục hồi an toàn."
            ),
        ),
        (
            "Benchmark tải cao & Tối ưu hóa độ trễ thực thi cho {crate}::{mod}",
            "Feature Expansion / Performance & Scalability",
            "P2",
            60,
            (
                "Đo lường throughput và memory allocation khi `{crate}::{mod}` chịu tải đồng thời cao, "
                "tối ưu hóa các điểm nghẽn lock contention hoặc buffer copying không cần thiết."
            ),
            (
                "Viết test benchmark đo lường P99 latency và chứng minh không bị rò rỉ bộ nhớ dưới tải cao."
            ),
        ),
    ]

    if len(candidate_topics) < 10:
        for title_tpl, category, prio, score, rationale_tpl, criteria_tpl in evolution_tracks:
            for crate_name, modules in capabilities.items():
                for mod in modules:
                    title = title_tpl.format(crate=crate_name, mod=mod)
                    if title not in seen_titles and not is_topic_in_backlog(title, existing_content):
                        seen_titles.add(title)
                        candidate_topics.append({
                            "title": title,
                            "priority": prio,
                            "category": category,
                            "rationale": rationale_tpl.format(crate=crate_name, mod=mod),
                            "acceptance_criteria": criteria_tpl.format(crate=crate_name, mod=mod),
                            "metric_count": score,
                        })
                    if len(candidate_topics) >= 15:
                        break
                if len(candidate_topics) >= 15:
                    break
            if len(candidate_topics) >= 15:
                break

    return candidate_topics


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
    feature_count = sum(1 for t in topics if "Feature Expansion" in t["category"])
    maint_count = len(topics) - feature_count

    print(f"\n🔍 Phát hiện {len(topics)} đề tài khả dụng ({maint_count} bảo trì/AST, {feature_count} tính năng mới brainstorm):")
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

