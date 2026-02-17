use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;

/// Serialize any value to JSON and send it over the WS connection.
pub async fn json<T: serde::Serialize>(
    tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    payload: &T,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(payload).unwrap_or_default();
    tx.send(Message::Text(json.into()))
        .await
        .map_err(axum::Error::new)
}
