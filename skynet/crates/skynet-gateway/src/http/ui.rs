use axum::response::Html;

static INDEX_HTML: &str = include_str!("../../static/index.html");

/// Serve the embedded web chat UI at `GET /`.
pub async fn ui_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}
