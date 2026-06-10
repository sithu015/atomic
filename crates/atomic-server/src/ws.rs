//! WebSocket endpoint for real-time event streaming

use crate::event_channel::EventChannel;
use crate::state::{AppState, ServerEvent};
use actix_web::{web, HttpRequest, HttpResponse};
use tokio::sync::broadcast;

/// WebSocket upgrade handler
/// Auth via query param: /ws?token=xxx
pub async fn ws_handler(
    req: HttpRequest,
    stream: web::Payload,
    state: web::Data<AppState>,
    events: EventChannel,
    query: web::Query<WsQuery>,
) -> Result<HttpResponse, actix_web::Error> {
    // Authenticate via query param
    let core = state
        .manager
        .active_core()
        .await
        .map_err(|_| actix_web::error::ErrorInternalServerError("Failed to get database"))?;
    match core.verify_api_token(&query.token).await {
        Ok(Some(_)) => {}
        _ => return Ok(HttpResponse::Unauthorized().finish()),
    }

    let (response, mut session, _msg_stream) = actix_ws::handle(&req, stream)?;

    // Subscribe to the request's event channel — AppState's process-wide
    // channel unless a composing layer injected one, in which case this
    // client streams the same channel that request-driven events publish to.
    let mut rx = events.0.subscribe();

    // Spawn task to forward broadcast events to this WebSocket client
    actix_web::rt::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(json) = serde_json::to_string(&event) {
                        if session.text(json).await.is_err() {
                            break; // Client disconnected
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("WebSocket client lagged, skipped {} events", n);
                    let event = ServerEvent::EventsLagged { skipped: n };
                    if let Ok(json) = serde_json::to_string(&event) {
                        if session.text(json).await.is_err() {
                            break;
                        }
                    }
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Ok(response)
}

#[derive(serde::Deserialize)]
pub struct WsQuery {
    pub token: String,
}
