pub mod assets;
pub mod benchmark;
pub mod cache;
pub mod renderer;
pub mod service;
pub mod sharded;
pub mod types;

pub use assets::{DashboardAsset, DashboardAssetRegistry};
pub use benchmark::{DashboardBenchmarkHarness, DashboardBenchmarkStats};
pub use cache::{
    ConcurrentDashboardCache, DashboardCacheEntry, DashboardCacheStats, ShardedDashboardCache,
};
pub use renderer::{render_template, render_with_fallback, FALLBACK_DASHBOARD_HTML};
pub use service::ResilientDashboardService;
pub use sharded::ShardedResilientDashboardService;
pub use types::{
    simulate_dashboard_fault, DashboardContext, DashboardError, DashboardErrorTier, DashboardFault,
    DashboardFaultPolicy, DashboardState,
};

pub const DASHBOARD_HTML: &str = include_str!("../static/dashboard.html");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashboard_html_not_empty() {
        assert!(!DASHBOARD_HTML.is_empty());
    }

    #[test]
    fn test_dashboard_html_contains_core_tags_and_elements() {
        let html_lower = DASHBOARD_HTML.to_lowercase();
        assert!(
            html_lower.contains("doctype"),
            "DASHBOARD_HTML should contain doctype"
        );
        assert!(
            html_lower.contains("html"),
            "DASHBOARD_HTML should contain html"
        );
        assert!(
            DASHBOARD_HTML.contains("auth-bar"),
            "DASHBOARD_HTML should contain auth-bar"
        );
        assert!(
            DASHBOARD_HTML.contains("stats-grid"),
            "DASHBOARD_HTML should contain stats-grid"
        );
        assert!(
            html_lower.contains("script"),
            "DASHBOARD_HTML should contain script"
        );
    }

    #[test]
    fn test_fallback_dashboard_html_valid_and_contains_core_elements() {
        let lower = FALLBACK_DASHBOARD_HTML.to_lowercase();
        assert!(lower.contains("<!doctype html>"));
        assert!(lower.contains("emergency fallback"));
        assert!(FALLBACK_DASHBOARD_HTML.contains("auth-bar"));
        assert!(FALLBACK_DASHBOARD_HTML.contains("stats-grid"));
    }
}
