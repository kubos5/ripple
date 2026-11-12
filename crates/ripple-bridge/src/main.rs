//! Development bridge.
//!
//! Serves the same request/event API the desktop shell uses, but over a
//! localhost WebSocket, so the UI can run in an ordinary browser tab
//! (`npm run dev`) against the real Rust core. Demo data by default.
//!
//!   cargo run -p ripple-bridge              # demo backend
//!   cargo run -p ripple-bridge -- --live    # real backend (build with --features live)

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Response, StatusCode, header};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use ripple_core::Ripple;
use ripple_core::api::Request;
use serde::Deserialize;
use serde_json::{Value, json};

const UI_ORIGINS: [&str; 4] =
    ["http://localhost:1420", "http://127.0.0.1:1420", "http://localhost:4173", "http://127.0.0.1:4173"];

#[derive(Deserialize)]
struct Call {
    id: u64,
    req: Value,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let live = std::env::args().any(|a| a == "--live");
    let dir = std::env::var("RIPPLE_DATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("ripple-bridge"));
    let ripple = Ripple::open(dir.clone(), !live).await?;
    let port: u16 = std::env::var("RIPPLE_BRIDGE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(1421);
    let app = Router::new()
        .route("/ws", get(ws))
        .route("/media/{*token}", get(media))
        .route("/health", get(|| async { "ok" }))
        .with_state(ripple);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    log::info!("ripple bridge on http://{addr} ({}, data in {})", if live { "live" } else { "demo" }, dir.display());
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}

fn origin_ok(headers: &HeaderMap) -> bool {
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(o) => UI_ORIGINS.contains(&o),
    }
}

async fn ws(State(ripple): State<Arc<Ripple>>, headers: HeaderMap, up: WebSocketUpgrade) -> Response<Body> {
    if !origin_ok(&headers) {
        return Response::builder().status(StatusCode::FORBIDDEN).body(Body::empty()).unwrap();
    }
    up.on_upgrade(move |socket| session(socket, ripple))
}

async fn session(socket: WebSocket, ripple: Arc<Ripple>) {
    let (mut tx, mut rx) = socket.split();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut events = ripple.subscribe();

    // One writer: replies and batched events share the socket.
    let writer = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if tx.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    let ev_tx = out_tx.clone();
    let pump = tokio::spawn(async move {
        loop {
            let first = match events.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = ev_tx.send(json!({ "events": [{ "type": "reset" }] }).to_string());
                    continue;
                }
                Err(_) => break,
            };
            // Drain whatever else is already queued so the UI gets one batch per burst.
            let mut batch = vec![first];
            while batch.len() < 512 {
                match events.try_recv() {
                    Ok(e) => batch.push(e),
                    Err(_) => break,
                }
            }
            if ev_tx.send(json!({ "events": batch }).to_string()).is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = rx.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(call) = serde_json::from_str::<Call>(&text) else { continue };
        let (ripple, out) = (ripple.clone(), out_tx.clone());
        tokio::spawn(async move {
            let reply = match serde_json::from_value::<Request>(call.req) {
                Ok(req) => match ripple.handle(req).await {
                    Ok(value) => json!({ "id": call.id, "ok": true, "value": value }),
                    Err(e) => json!({ "id": call.id, "ok": false, "error": format!("{e}") }),
                },
                Err(e) => json!({ "id": call.id, "ok": false, "error": format!("Bad request: {e}") }),
            };
            let _ = out.send(reply.to_string());
        });
    }
    pump.abort();
    writer.abort();
}

#[derive(Deserialize)]
struct MediaQuery {
    wav: Option<String>,
}

async fn media(
    State(ripple): State<Arc<Ripple>>,
    Path(token): Path<String>,
    Query(q): Query<MediaQuery>,
    headers: HeaderMap,
) -> Response<Body> {
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(String::from);
    let wav = q.wav.is_some();
    let served = tokio::task::spawn_blocking(move || ripple.serve_media(&token, range.as_deref(), wav)).await;
    let cors = "*";
    match served {
        Ok(Ok(s)) => {
            let mut r = Response::builder()
                .status(s.status)
                .header(header::CONTENT_TYPE, if wav { "audio/wav" } else { s.mime })
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, cors)
                .header(header::CONTENT_LENGTH, s.body.len());
            if s.status == 206 {
                r = r.header(header::CONTENT_RANGE, format!("bytes {}-{}/{}", s.start, s.end, s.total));
            }
            r.body(Body::from(s.body)).unwrap()
        }
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, cors)
            .body(Body::empty())
            .unwrap(),
    }
}
