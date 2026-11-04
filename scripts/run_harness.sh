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
chmod +x harness/evaluators/benchmark.py harness/memory_bridge.py harness/orchestrator.py harness/researcher.py 2>/dev/null || true

MODE="${1:---all}"

case "${MODE}" in
  --discover|-d)
    # Tự động quét mã nguồn và sinh đề tài nghiên cứu mới vào RESEARCH_BACKLOG.md
    python3 harness/topic_generator.py "${@:2}"
    ;;
  --research|-r)
    # Kích hoạt Agent tự nghiên cứu theo đề tài trong RESEARCH_BACKLOG.md
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
  --update|-u)
    # Cập nhật release nhị phân hệ thống
    python3 harness/orchestrator.py --update
    ;;
  --all|*)
    # Vòng lặp thẩm định & gatekeeper
    python3 harness/orchestrator.py
    ;;
esac
