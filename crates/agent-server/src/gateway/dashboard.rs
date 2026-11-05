use axum::response::Html;

/// Trả về toàn bộ giao diện HTML Web Dashboard quản lý tài khoản và Gateway
pub async fn dashboard_handler() -> Html<&'static str> {
    Html(crate::dashboard_html::DASHBOARD_HTML)
}
