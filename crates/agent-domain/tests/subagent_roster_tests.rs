use agent_domain::{
    default_subagent_roster, discover_workspace_roster, load_subagents_from_dir,
    roster_to_rig_spec, SubagentFrontmatter, SubagentSpec,
};
use std::path::PathBuf;

struct TempRosterDir(PathBuf);

impl TempRosterDir {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "tuzygent-subagent-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }
}

impl Drop for TempRosterDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_subagent_markdown_and_serde_roundtrip() {
    let md = "---\r\nname: Repo Scout\r\ndescription: Fast read-only codebase finder\r\ntools: Read, Grep, Glob\r\nmodel: sonnet\r\n---\r\n\r\nLocate symbols and cite exact line numbers.";
    let spec = SubagentSpec::from_markdown_str("fallback", md, Some("agents/repo-scout.md"))
        .expect("parse markdown frontmatter");

    assert_eq!(spec.id, "repo-scout");
    assert_eq!(spec.name, "Repo Scout");
    assert_eq!(spec.description, "Fast read-only codebase finder");
    assert_eq!(spec.capabilities, vec!["Read", "Grep", "Glob"]);
    assert_eq!(spec.model.as_deref(), Some("sonnet"));
    assert_eq!(spec.harness, "process");
    assert_eq!(spec.autonomy, "full");
    assert_eq!(
        spec.system_prompt,
        "Locate symbols and cite exact line numbers."
    );

    // JSON round-trips for both serialized types
    let json = serde_json::to_string(&spec).expect("serialize SubagentSpec");
    let decoded: SubagentSpec = serde_json::from_str(&json).expect("deserialize SubagentSpec");
    assert_eq!(decoded, spec);

    let fm = SubagentFrontmatter {
        id: Some("sec".to_string()),
        name: Some("Security".to_string()),
        description: Some("Audit".to_string()),
        role: None,
        capabilities: vec!["audit".to_string()],
        tools: vec!["Read".to_string()],
        engine: Some("codex".to_string()),
        model: None,
        harness: None,
        autonomy: None,
    };
    let fm_json = serde_json::to_string(&fm).expect("serialize SubagentFrontmatter");
    let fm_dec: SubagentFrontmatter =
        serde_json::from_str(&fm_json).expect("deserialize SubagentFrontmatter");
    assert_eq!(fm_dec, fm);

    // Markdown round-trip & seat adapter
    let rendered = spec.to_markdown().expect("render markdown");
    let reparsed =
        SubagentSpec::from_markdown_str("fallback", &rendered, spec.source_path.as_deref())
            .expect("reparse rendered markdown");
    assert_eq!(reparsed, spec);

    let seat = spec.to_seat_config();
    assert_eq!(seat.seat_id, "repo-scout");
    assert_eq!(seat.persona_path.as_deref(), Some("agents/repo-scout.md"));
}

#[test]
fn test_discover_workspace_roster_uses_agents_md_claude_dir_or_builtin_fallback() {
    // 1. Empty workspace falls back to built-in 3-specialist roster with no static channels
    let empty_ws = TempRosterDir::new("empty");
    let (builtin_specs, builtin_rig) =
        discover_workspace_roster(&empty_ws.0).expect("discover fallback roster");
    assert_eq!(builtin_specs, default_subagent_roster());
    assert_eq!(builtin_specs.len(), 3);
    assert!(builtin_rig.channels.is_empty());
    assert_eq!(builtin_rig.seats.len(), 3);

    // 2. `.claude/agents/*.md` is discovered automatically
    let claude_ws = TempRosterDir::new("claude");
    let claude_dir = claude_ws.0.join(".claude").join("agents");
    std::fs::create_dir_all(&claude_dir).expect("create .claude/agents");
    std::fs::write(
        claude_dir.join("auditor.md"),
        "---\nname: Security Auditor\ndescription: Finds auth flaws\ncapabilities:\n  - security\n  - review\n---\nAudit auth boundaries carefully.\n",
    )
    .expect("write auditor.md");

    let loaded = load_subagents_from_dir(&claude_dir).expect("load dir");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "security-auditor");

    let (discovered, rig) = discover_workspace_roster(&claude_ws.0).expect("discover claude dir");
    assert_eq!(discovered.len(), 1);
    assert!(rig.channels.is_empty());
    assert_eq!(rig.seats[0].seat_id, "security-auditor");

    let built = roster_to_rig_spec("custom-id", "Custom Roster", &discovered);
    assert_eq!(built.rig.id, "custom-id");
    assert_eq!(built.seats.len(), 1);
}
