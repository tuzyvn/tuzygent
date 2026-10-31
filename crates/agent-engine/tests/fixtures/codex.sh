#!/bin/sh
# Deterministic Codex JSONL protocol fixture. No model or network access.
if [ -n "${CODEX_TEST_ARGS:-}" ]; then printf '%s\n' "$@" > "$CODEX_TEST_ARGS"; fi
for last do :; done
session=12345678-1234-4234-8234-123456789abc
printf '{"type":"thread.started","thread_id":"%s"}\n' "$session"
case "${CODEX_TEST_CASE:-success}" in
    fail) printf '%s\n' '{"type":"turn.failed","error":{"message":"model rejected request"}}'; exit 0 ;;
    invalid) printf '%s\n' 'not-json'; exit 0 ;;
    empty) printf '%s\n' '{"type":"turn.completed"}'; exit 0 ;;
    exit) printf '%s\n' 'authentication failed' >&2; exit 7 ;;
    timeout) sleep 3; exit 0 ;;
    transient) printf '%s\n' '{"type":"error","message":"retrying connection"}' ;;
esac
text=CODEX_DELEGATED
if [ "${CODEX_TEST_CASE:-}" = ooda ]; then
    case "$last" in
        *'Autonomous AI System Architect and Planner'*)
            text='[{\"id\":\"ST-1\",\"title\":\"delegate\",\"command_or_prompt\":\"do the delegated task\",\"expected_verification\":\"done\"}]' ;;
    esac
fi
printf '%s\n' '{"type":"item.completed","item":{"type":"agent_message","phase":"commentary","text":"working"}}'
printf '{"type":"item.completed","item":{"type":"agent_message","phase":"final_answer","text":"%s"}}\n' "$text"
if [ "${CODEX_TEST_CASE:-}" != partial ]; then
    printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":12,"output_tokens":3}}'
fi
