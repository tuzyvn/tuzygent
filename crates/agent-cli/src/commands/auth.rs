use crate::cli::AuthAction;

pub async fn handle(action: AuthAction) -> anyhow::Result<()> {
    let vault = agent_engine::AccountVault::new();
    match action {
        AuthAction::List { engine } => {
            let eng = engine
                .as_deref()
                .map(agent_engine::AccountEngineKind::from_str_name);
            let accounts = vault.list_accounts(eng.as_ref());
            if accounts.is_empty() {
                println!(
                    "ℹ️ Không có tài khoản nào được đăng ký trong kho (~/.agentcore/accounts/)."
                );
            } else {
                println!("📋 DANH SÁCH TÀI KHOẢN CLI (ACCOUNT VAULT):");
                println!(
                    "{:<14} | {:<20} | {:<24} | {:<16} | {:<8}",
                    "ENGINE", "ID", "TÊN", "TRẠNG THÁI", "REQUESTS"
                );
                println!(
                    "{:-<14}-+-{:-<20}-+-{:-<24}-+-{:-<16}-+-{:-<8}",
                    "", "", "", "", ""
                );
                for acc in accounts {
                    let status_str = match &acc.status {
                        agent_engine::AccountStatus::Ready => "Ready".to_string(),
                        agent_engine::AccountStatus::Cooldown { until, reason } => {
                            let remaining = (*until - chrono::Utc::now()).num_seconds().max(0);
                            format!("Cooldown({}s, {})", remaining, reason)
                        }
                        agent_engine::AccountStatus::Disabled => "Disabled".to_string(),
                    };
                    println!(
                        "{:<14} | {:<20} | {:<24} | {:<16} | {:<8}",
                        acc.engine.to_string(),
                        acc.id,
                        acc.name,
                        status_str,
                        acc.requests_count
                    );
                }
            }
        }
        AuthAction::Add {
            engine,
            id,
            name,
            api_key,
        } => {
            let engine_kind = agent_engine::AccountEngineKind::from_str_name(&engine);
            let mut extra_env = std::collections::HashMap::new();
            if let Some(key) = api_key {
                extra_env.insert("OPENAI_API_KEY".to_string(), key.clone());
                extra_env.insert("CODEX_API_KEY".to_string(), key);
            }
            match vault.register_account(engine_kind, &id, &name, extra_env) {
                Ok(acc) => {
                    println!(
                        "✅ Đã đăng ký thành công tài khoản '{}' cho engine '{}'!",
                        acc.id, acc.engine
                    );
                    println!("📁 Thư mục Sandbox cách ly: {}", acc.home_dir.display());
                }
                Err(e) => {
                    eprintln!("❌ Lỗi khi đăng ký tài khoản: {}", e);
                }
            }
        }
        AuthAction::Remove { engine, id } => {
            let engine_kind = agent_engine::AccountEngineKind::from_str_name(&engine);
            match vault.remove_account(&engine_kind, &id) {
                Ok(true) => println!(
                    "🗑️ Đã xóa tài khoản '{}' của engine '{}' thành công.",
                    id, engine_kind
                ),
                Ok(false) => println!("⚠️ Không tìm thấy tài khoản '{}' để xóa.", id),
                Err(e) => eprintln!("❌ Lỗi: {}", e),
            }
        }
    }

    Ok(())
}
