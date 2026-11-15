#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AGENT_CTL="${AGENT_CTL:-$(command -v agent-ctl || echo "$ROOT_DIR/target/release/agent-ctl")}"
STACKPILOT_URL="${STACKPILOT_URL:-http://127.0.0.1:49382}"
STACKPILOT_MCP="${STACKPILOT_MCP:-}"

if [[ ! -x "$AGENT_CTL" ]]; then
  echo "Missing agent-ctl (install with: cargo install --path crates/agent-cli)" >&2
  exit 1
fi

echo "==> 1. Verifying tuzygent CLI & Markdown Subagent Roster ($AGENT_CTL)"
"$AGENT_CTL" rig status

if [[ -n "$STACKPILOT_MCP" && -f "$STACKPILOT_MCP" ]]; then
  echo ""
  echo "==> 2. Verifying StackPilot MCP Server ($STACKPILOT_MCP)"
  printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"tuzygent-smoke","version":"0.1.0"}}}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"stackpilot_list_environments","arguments":{}}}' \
    | node "$STACKPILOT_MCP"
else
  echo ""
  echo "==> 2. Verifying StackPilot HTTP API ($STACKPILOT_URL/api/environments)"
  curl -fsS "$STACKPILOT_URL/api/environments" | head -c 500
  echo ""
fi
