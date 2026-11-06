use axum::http::StatusCode;

use super::types::{DashboardContext, DashboardError, DashboardFault, DashboardFaultPolicy};

/// Minimal, self-contained, guaranteed valid fallback HTML used when primary template rendering fails.
pub const FALLBACK_DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>AgentCore Dashboard (Emergency Fallback)</title>
  <style>
    body { background-color: #0f172a; color: #f8fafc; font-family: -apple-system, BlinkMacSystemFont, sans-serif; padding: 24px; }
    .emergency-banner { background: #7f1d1d; border: 1px solid #ef4444; border-radius: 8px; padding: 16px; margin-bottom: 24px; }
    .auth-bar { background: #1e293b; border: 1px solid #334155; border-radius: 8px; padding: 12px; margin-bottom: 16px; }
    .stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 12px; }
    .stat-card { background: #1e293b; padding: 12px; border-radius: 6px; }
  </style>
</head>
<body>
  <div class="emergency-banner">
    <h2>⚠️ Emergency Fallback Mode</h2>
    <p>The primary dashboard template could not be rendered or was disrupted. Core monitoring elements remain accessible below.</p>
  </div>
  <div class="auth-bar">
    <span>🔑 API Token: [Emergency Mode Active]</span>
  </div>
  <div class="stats-grid">
    <div class="stat-card">Status: Degradation Active</div>
    <div class="stat-card">Fallback Version: 1.0.0-fallback</div>
  </div>
  <script>
    console.warn("AgentCore Dashboard is running in emergency fallback mode.");
  </script>
</body>
</html>"#;

/// Validates syntax and substitutes placeholder tags such as `{{KEY}}` from context.
pub fn render_template(template: &str, ctx: &DashboardContext) -> Result<String, DashboardError> {
    // 1. Syntax integrity check: verify delimiter pairing
    let mut cursor = 0;
    while let Some(open_pos) = template[cursor..].find("{{") {
        let actual_open = cursor + open_pos;
        let sub = &template[actual_open..];
        match sub.find("}}") {
            Some(close_pos) => {
                cursor = actual_open + close_pos + 2;
            }
            None => {
                return Err(DashboardError::TemplateRenderFailure(format!(
                    "Unclosed placeholder delimiter starting at index {}",
                    actual_open
                )));
            }
        }
    }

    let mut rendered = template.to_string();

    // 2. Perform known contextual substitutions
    if let Some(title) = &ctx.title {
        rendered = rendered.replace("{{title}}", title);
        rendered = rendered.replace("{{TITLE}}", title);
    }

    if let Some(gw) = &ctx.gateway_url {
        rendered = rendered.replace("{{gateway_url}}", gw);
        rendered = rendered.replace("{{GATEWAY_URL}}", gw);
    }

    if let Some(ver) = &ctx.version {
        rendered = rendered.replace("{{version}}", ver);
        rendered = rendered.replace("{{VERSION}}", ver);
    }

    if let Some(token) = &ctx.auth_token_preset {
        rendered = rendered.replace("{{auth_token_preset}}", token);
        rendered = rendered.replace("{{AUTH_TOKEN_PRESET}}", token);
    }

    if let Some(banner) = &ctx.custom_banner {
        rendered = rendered.replace("{{custom_banner}}", banner);
        rendered = rendered.replace("{{CUSTOM_BANNER}}", banner);
    }

    // 3. Perform arbitrary dynamic substitutions
    for (k, v) in &ctx.extra_variables {
        let tag = format!("{{{{{}}}}}", k);
        rendered = rendered.replace(&tag, v);
    }

    Ok(rendered)
}

/// Renders the dashboard with graceful degradation to `FALLBACK_DASHBOARD_HTML` upon failure.
/// Returns `(html_body, http_status_code, is_fallback_flag)`.
pub fn render_with_fallback(
    template: &str,
    ctx: &DashboardContext,
    fault_policy: Option<&mut DashboardFaultPolicy>,
) -> (String, StatusCode, bool) {
    if let Some(policy) = fault_policy {
        if let Some(fault) = policy.should_inject() {
            if fault == DashboardFault::TemplateRenderFailure {
                return (
                    FALLBACK_DASHBOARD_HTML.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                    true,
                );
            }
        }
    }

    match render_template(template, ctx) {
        Ok(rendered) => (rendered, StatusCode::OK, false),
        Err(_) => (
            FALLBACK_DASHBOARD_HTML.to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
            true,
        ),
    }
}
