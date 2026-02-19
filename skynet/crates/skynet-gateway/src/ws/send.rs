use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;

/// Shared, concurrently writable WS sink.
pub type SharedSink = Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>;

/// Serialize any value to JSON and send it over an exclusively-borrowed sink.
///
/// Used for auth and fast synchronous methods where the caller already holds
/// a `&mut` reference to the sink (e.g. via `Mutex::lock()`).
pub async fn json<T: serde::Serialize>(
    tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    payload: &T,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(payload).unwrap_or_default();
    tx.send(Message::Text(json.into()))
        .await
        .map_err(axum::Error::new)
}

/// Serialize any value to JSON and send it over a shared sink.
///
/// Used by spawned chat tasks and connection loop ticks where multiple
/// concurrent writers may exist.
pub async fn json_shared<T: serde::Serialize>(
    tx: &SharedSink,
    payload: &T,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(payload).unwrap_or_default();
    tx.lock()
        .await
        .send(Message::Text(json.into()))
        .await
        .map_err(axum::Error::new)
}
