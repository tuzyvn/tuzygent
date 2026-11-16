use agent_cli::commands::mcp::{build_tuzygent_mcp_gateway, run_stdio_loop};
use agent_cli::{Cli, Commands};
use serde_json::{json, Value};
use std::io::Cursor;

#[test]
fn test_cli_parses_mcp_subcommand() {
    let parsed = Cli::try_parse_resilient(["agent-ctl", "mcp", "--workspace", "/tmp/ws"])
        .expect("should parse agent-ctl mcp");
    assert!(matches!(
        parsed.command,
        Commands::Mcp { workspace } if workspace == "/tmp/ws"
    ));
}

#[test]
fn test_mcp_stdio_server_handles_initialize_list_and_dry_run_calls() {
    let mut gateway = build_tuzygent_mcp_gateway(".");

    let requests = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "tuzygent_rig_list_targets",
                "arguments": {}
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "tuzygent_workflow_dry_run",
                "arguments": {
                    "workflow": {
                        "id": "wf-test",
                        "name": "Dry Run Test",
                        "nodes": [
                            {"id": "t1", "type": "triggerNode", "data": {"outputContract": "text"}},
                            {"id": "n1", "type": "toolNode", "data": {"command": "echo ok", "inputContract": "text"}}
                        ],
                        "edges": [
                            {"id": "e1", "source": "t1", "target": "n1"}
                        ]
                    }
                }
            }
        }),
    ];

    let input_buf = requests
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let mut output_buf = Vec::new();

    run_stdio_loop(&mut gateway, Cursor::new(input_buf), &mut output_buf)
        .expect("stdio loop should succeed");

    let output_str = String::from_utf8(output_buf).expect("valid utf8");
    let lines: Vec<Value> = output_str
        .lines()
        .map(|l| serde_json::from_str(l).expect("valid json line"))
        .collect();

    // 4 responses (notification produces no response)
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0]["id"], 1);
    assert_eq!(
        lines[0]["result"]["serverInfo"]["name"],
        "tuzygent-mcp-gateway"
    );

    assert_eq!(lines[1]["id"], 2);
    let tools = lines[1]["result"]["tools"].as_array().expect("tools array");
    assert!(tools
        .iter()
        .any(|t| t["name"] == "tuzygent_rig_list_targets"));
    assert!(tools
        .iter()
        .any(|t| t["name"] == "tuzygent_workflow_dry_run"));

    assert_eq!(lines[2]["id"], 3);
    let targets_text = lines[2]["result"]["content"][0]["text"]
        .as_str()
        .expect("targets text");
    assert!(targets_text.contains("snapshot_id"));

    assert_eq!(lines[3]["id"], 4);
    let dry_run_text = lines[3]["result"]["content"][0]["text"]
        .as_str()
        .expect("dry_run text");
    let dry_run_json: Value = serde_json::from_str(dry_run_text).expect("valid DryRunReport JSON");
    assert_eq!(dry_run_json["valid"], true);
    assert_eq!(dry_run_json["execution_order"], json!(["t1", "n1"]));
}
