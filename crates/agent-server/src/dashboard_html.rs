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
        assert!(html_lower.contains("doctype"), "DASHBOARD_HTML should contain doctype");
        assert!(html_lower.contains("html"), "DASHBOARD_HTML should contain html");
        assert!(DASHBOARD_HTML.contains("auth-bar"), "DASHBOARD_HTML should contain auth-bar");
        assert!(DASHBOARD_HTML.contains("stats-grid"), "DASHBOARD_HTML should contain stats-grid");
        assert!(html_lower.contains("script"), "DASHBOARD_HTML should contain script");
    }
}
