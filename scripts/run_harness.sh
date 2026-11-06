#!/usr/bin/env bash
# ==============================================================================
# Tuzygent Continuous Autonomous Research & Self-Update Harness
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${WORKSPACE_ROOT}"

echo "=========================================================="
echo " Starting Tuzygent Continuous Evolution & Research Harness"
echo " Workspace: ${WORKSPACE_ROOT}"
echo " Time     : $(date)"
echo "=========================================================="

# Ensure execution bits
chmod +x harness/evaluators/benchmark.py harness/memory_bridge.py harness/orchestrator.py harness/researcher.py harness/topic_generator.py harness/version_manager.py 2>/dev/null || true

MODE="${1:---all}"

case "${MODE}" in
  --continuous|-c)
    # Vòng lặp tự nghiên cứu liên tục xuyên suốt (Autonomous Continuous Pipeline)
    python3 harness/researcher.py --continuous "${@:2}"
    ;;
  --discover|-d)
    # Tự động quét mã nguồn và sinh đề tài nghiên cứu mới vào RESEARCH_BACKLOG.md
    python3 harness/topic_generator.py "${@:2}"
    ;;
  --brainstorm)
    # Tự động brainstorm đề tài tính năng mới dựa trên năng lực hệ thống hiện có
    python3 harness/topic_generator.py --brainstorm "${@:2}"
    ;;
  --research|-r)
    # Kích hoạt Agent tự nghiên cứu (hỗ trợ --continuous, --max-topics N)
    python3 harness/researcher.py "${@:2}"
    ;;
  --benchmark|-b)
    # Chỉ đo lường benchmark & scorecard
    python3 harness/evaluators/benchmark.py
    ;;
  --lessons|-l)
    # Liệt kê bài học kinh nghiệm trong bộ nhớ
    python3 harness/memory_bridge.py list
    ;;
  --update|-u|--release|--push)
    # Tăng version, commit, cập nhật release nhị phân hệ thống và push code lên git
    python3 harness/orchestrator.py --update "${@:2}"
    ;;
  --bump)
    # Gọi trực tiếp bộ quản lý version & push
    python3 harness/version_manager.py "${@:2}"
    ;;
  --all|*)
    # Vòng lặp thẩm định & gatekeeper (Pass benchmark -> Tăng version + Commit + Update binary + Push)
    python3 harness/orchestrator.py "${@:2}"
    ;;
esac
