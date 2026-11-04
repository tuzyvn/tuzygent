# Tuzygent Continuous Evolution & Research Harness

A continuous benchmarking, evaluation, and autonomous self-updating harness designed to research, stress-test, and incrementally refine the Tuzygent Multi-Agent Enterprise Platform.

## 🏗️ Architecture

```
┌────────────────────────────────────────────────────────┐
│                   Harness Orchestrator                 │
└────────────────────────────────────────────────────────┘
          │
          ├── [1] Evaluator & Benchmark (harness/evaluators/benchmark.py)
          │       - Cargo tests (53+ unit & integration tests)
          │       - Cargo Clippy (-D warnings)
          │       - Operational Scenarios (harness/scenarios/*.json)
          │
          ├── [2] Collective Memory Bridge (harness/memory_bridge.py)
          │       - Reads/writes to `storage/harness_memory/harness_lessons.json`
          │       - Adheres to `HarnessLesson` schema from `agent-runtime::harness`
          │       - Remembers failure patterns and prevents regressions
          │
          └── [3] Release Gatekeeper & Self-Update (`agent-ctl update`)
                  - Verifies 100% passing criteria
                  - Builds atomic release binary
                  - Replaces active system binary
```

## 🚀 Quick Usage

### 1. Run Complete Evaluation & Self-Update
```bash
./scripts/run_harness.sh
```

### 2. Run Benchmark Only
```bash
./scripts/run_harness.sh --benchmark
```

### 3. List Accumulated Lessons from Memory
```bash
./scripts/run_harness.sh --lessons
```

### 4. Force System Update Verification
```bash
./scripts/run_harness.sh --update
```

## 📋 Scenarios
- `01_security_boundary.json`: Validates sandbox boundaries and secret redaction.
- `02_account_rotation.json`: Tests model registry resolution and account exhaustion handling.
- `03_memory_persistence.json`: Verifies schema isolation and atomic memory writes.
- `04_cli_version_update.json`: Checks CLI version output and update command readiness.
