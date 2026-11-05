use agent_engine::config::EnginesConfig;
use agent_engine::{EngineInput, EngineRouter};
use agent_runtime::SandboxConfig;
use std::path::Path;
use std::time::Instant;

pub async fn handle(workspace: String, test: Option<String>) -> anyhow::Result<()> {
    let ws_path = Path::new(&workspace);
    let config = EnginesConfig::load_hierarchical(Some(ws_path));
    let detected_driver = SandboxConfig::detect_best_driver();

    println!("============================================================");
    println!("🛠️  TUZYGENT OS AI CORE ENGINES & SANDBOX CONFIGURATION");
    println!("============================================================");
    println!("📌 Default Engine:        {}", config.defaults.engine);
    println!(
        "⏱️  Default Timeout:       {}s",
        config.defaults.timeout_seconds
    );
    println!(
        "🛡️  Configured Sandbox:   {}",
        config.defaults.sandbox_driver
    );
    println!("🔍 Detected OS Sandbox:   {}", detected_driver);
    println!("\n📦 Registered Engines ({}):", config.engines.len());
    for (id, eng) in &config.engines {
        println!("  • [{}] (kind: {})", id, eng.kind);
        if let Some(ref bin) = eng.binary_path {
            println!("      Binary: {}", bin);
        }
        println!("      Args:   {:?}", eng.args);
        if let Some(ref m) = eng.model {
            println!("      Model:  {}", m);
        }
        if let Some(ref fb) = eng.interactive_fallback {
            println!("      Interactive Fallback: {}", fb);
        }
    }

    println!("\n🔐 Allowed Credential Passthrough:");
    for cred in &config.security.mount_credentials {
        println!(
            "  • {} -> {} (ro: {})",
            cred.source, cred.target, cred.readonly
        );
    }
    println!("  Env Whitelist: {:?}", config.security.allowed_env_vars);

    if let Some(target_engine) = test {
        println!("\n🧪 Testing execution on engine [{}]...", target_engine);
        let test_input = EngineInput {
            prompt_or_command: "Respond with 'Hello from Tuzygent OS AI Core' in one line"
                .to_string(),
            system_context: String::new(),
            working_dir: workspace.clone(),
            timeout_seconds: 15,
            on_stream_chunk: None,
            command_sandbox: None,
            on_event: None,
            temperature: None,
            max_tokens: None,
            max_output_bytes: None,
        };

        let router = EngineRouter::from_config(&config);
        let start = Instant::now();
        match router
            .dispatch_by_name(Some(&target_engine), &test_input)
            .await
        {
            Ok(out) => {
                println!(
                    "  ✅ Success in {}ms (exit code: {})",
                    out.execution_duration_ms, out.exit_code
                );
                println!("  📄 Output:\n{}", out.stdout_content.trim());
            }
            Err(e) => {
                println!(
                    "  ❌ Test failed in {}ms: {}",
                    start.elapsed().as_millis(),
                    e
                );
            }
        }
    }
    println!("============================================================");

    Ok(())
}
