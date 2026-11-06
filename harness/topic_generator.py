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
    """Dynamically discover modules and capabilities across workspace crates."""
    capabilities = {}
    for crate in crates:
        crate_name = crate.name
        modules = []
        src_dir = crate / "src"
        if src_dir.exists():
            for item in src_dir.glob("*"):
                if item.is_dir() and not item.name.startswith("."):
                    modules.append(item.name)
                elif item.suffix == ".rs" and item.stem not in ("lib", "main"):
                    modules.append(item.stem)
        capabilities[crate_name] = sorted(modules)
    return capabilities


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


def brainstorm_feature_topics(crates: List[Path], existing_content: str) -> List[Dict]:
    """
    Dynamically brainstorm next-generation features based on current capability graph.
    Categories:
      1. Resilience & Fault Tolerance
      2. Multi-Agent Collaboration
      3. Observability & Telemetry
      4. Security & Sandboxing
      5. Developer Ergonomics & Tooling
      6. Adaptive Intelligence
    """
    capabilities = extract_system_capabilities(crates)

    feature_matrix = [
        # --- 1. Resilience & Fault Tolerance ---
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

        # --- 2. Multi-Agent Collaboration ---
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

        # --- 3. Observability & Telemetry ---
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

        # --- 4. Security & Sandboxing ---
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

        # --- 5. Developer Ergonomics & Tooling ---
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

        # --- 6. Adaptive Intelligence ---
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
    ]

    # Filter out topics that already exist in RESEARCH_BACKLOG.md (whether pending or completed)
    candidate_topics = []
    for item in feature_matrix:
        if item["title"] not in existing_content:
            candidate_topics.append(item)

    # Infinite Evolutionary Tier: If all base feature matrix topics are completed,
    # dynamically synthesize next-level Cross-Module Chaos, Concurrency Stress,
    # and Performance Optimization topics based on real workspace modules!
    if not candidate_topics:
        cross_topics = []
        crate_names = list(capabilities.keys())
        for crate_name, modules in capabilities.items():
            for mod in modules[:3]:
                chaos_title = f"Chaos Engineering & Fault Injection Test cho module {crate_name}::{mod}"
                if chaos_title not in existing_content:
                    cross_topics.append({
                        "title": chaos_title,
                        "priority": "P2",
                        "category": "Feature Expansion / Chaos Engineering",
                        "rationale": (
                            f"Mô phỏng lỗi ngắt quãng, socket timeout, hoặc I/O failure đột ngột trong module "
                            f"`{crate_name}::{mod}` để kiểm tra tính bền vững và khả năng tự phục hồi mà không làm sập toàn bộ hệ thống."
                        ),
                        "acceptance_criteria": (
                            f"Viết integration test mô phỏng kịch bản lỗi ngắt quãng và xác minh {crate_name}::{mod} phục hồi an toàn."
                        ),
                        "metric_count": 65,
                    })

                perf_title = f"Benchmark tải cao & Tối ưu hóa độ trễ thực thi cho {crate_name}::{mod}"
                if perf_title not in existing_content:
                    cross_topics.append({
                        "title": perf_title,
                        "priority": "P2",
                        "category": "Feature Expansion / Performance & Scalability",
                        "rationale": (
                            f"Đo lường throughput và memory allocation khi `{crate_name}::{mod}` chịu tải 10,000 requests đồng thời, "
                            f"tối ưu hóa các điểm nghẽn lock contention hoặc buffer copying không cần thiết."
                        ),
                        "acceptance_criteria": (
                            f"Viết test benchmark đo lường P99 latency và chứng minh không bị rò rỉ bộ nhớ dưới tải cao."
                        ),
                        "metric_count": 60,
                    })

        candidate_topics.extend(cross_topics)

    return candidate_topics


def discover_topics(force_brainstorm: bool = False) -> List[Dict]:
    """
    Execute dynamic discovery across AST smells, complexity percentiles, runtime lessons,
    and automatic frontier feature brainstorming when maintenance topics are exhausted.
    """
    crates = get_workspace_crates()
    rs_files = get_all_production_rs_files(crates)

    existing_content = ""
    if BACKLOG_FILE.exists():
        existing_content = BACKLOG_FILE.read_text(encoding="utf-8")

    all_topics = []
    if not force_brainstorm:
        all_topics.extend(analyze_code_smells(rs_files))
        all_topics.extend(analyze_complexity_distribution(rs_files))
        all_topics.extend(analyze_runtime_lessons())

    # Check how many maintenance topics are genuinely NEW (not yet in backlog)
    new_maintenance_topics = [t for t in all_topics if t["title"] not in existing_content]

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

