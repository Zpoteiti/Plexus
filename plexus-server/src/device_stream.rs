//! Device-stream relay: bridges a client device's `ReadStream` WS frames
//! into an HTTP streamed response.
//!
//! Used by `GET /api/device-stream/{device_name}/{path:.*}` so the browser
//! can pull bytes from a connected device through the server without any
//! on-disk staging. Each hit opens a fresh WS stream, correlated by a
//! random `request_id`; chunks flow through an `mpsc` channel into
//! `axum::body::Body::from_stream`.
//!
//! Correlation lives in `AppState::streams`, populated by the handler and
//! drained by `ws::handle_device_session` when it receives
//! `ClientToServer::StreamChunk` / `StreamEnd` / `StreamError`.
//!
//! Timeout: `STREAM_IDLE_TIMEOUT` between chunks. On timeout, the HTTP body
//! is closed with a broken-pipe error and the correlation slot is cleaned
//! up so the client's late chunks are silently dropped.

use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Response, StatusCode};
use futures_util::stream::{Stream, unfold};
use plexus_common::errors::{ApiError, ErrorCode};
use plexus_common::mime::detect_mime_from_extension;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Max time to wait between chunks before aborting the stream.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Bounded channel for incoming stream frames — backpressures the client
/// if the HTTP consumer is slow.
const STREAM_CHANNEL_CAPACITY: usize = 32;

/// One correlated frame from a device-origin stream. Constructed by the
/// WS message loop (see `ws::handle_device_session`) and consumed by the
/// `device_stream` handler.
#[derive(Debug)]
pub enum StreamFrame {
    Chunk(Vec<u8>),
    End,
    Error(String),
}

/// RAII guard that removes the correlation slot from `AppState::streams`
/// when the handler's stream is dropped (normal end, client disconnect,
/// panic). Ensures late chunks from the device don't leak memory.
struct SlotGuard {
    state: Arc<AppState>,
    device_key: String,
    request_id: String,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        if let Some(slots) = self.state.streams.get(&self.device_key) {
            slots.remove(&self.request_id);
        }
    }
}

/// Handler for `GET /api/device-stream/{device_name}/{*path}`.
///
/// 1. Resolve user from JWT.
/// 2. Verify device ownership via DB — 404 on missing/not-yours.
/// 3. Require the device to be online — 503 on offline.
/// 4. Register a correlation slot, send `ReadStream`, pipe frames → body.
pub async fn device_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((device_name, path)): Path<(String, String)>,
) -> Result<Response<Body>, ApiError> {
    let claims = crate::auth::extract_claims(&headers, &state.config.jwt_secret)?;
    let user_id = claims.sub;

    // Ownership check — find_by_user_and_device scopes by user_id, so a
    // None result means either "no such device" or "not yours". Either
    // way, 404 is the right answer (don't leak existence).
    let _device = crate::db::devices::find_by_user_and_device(&state.db, &user_id, &device_name)
        .await
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("{e}")))?
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "Device not found"))?;

    let device_key = AppState::device_key(&user_id, &device_name);
    let conn = state
        .devices
        .get(&device_key)
        .ok_or_else(|| ApiError::new(ErrorCode::DeviceOffline, "Device is offline"))?;

    // Correlation slot. Bounded so a slow browser backpressures the
    // device via the WS sink's flow control.
    let request_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel::<StreamFrame>(STREAM_CHANNEL_CAPACITY);
    state
        .streams
        .entry(device_key.clone())
        .or_default()
        .insert(request_id.clone(), tx);
    let guard = SlotGuard {
        state: state.clone(),
        device_key: device_key.clone(),
        request_id: request_id.clone(),
    };

    // Send ReadStream on the device's sink. On send failure we drop the
    // guard (= slot cleanup) and return 503.
    let msg = plexus_common::protocol::ServerToClient::ReadStream {
        request_id: request_id.clone(),
        path: path.clone(),
    };
    let json = serde_json::to_string(&msg)
        .map_err(|e| ApiError::new(ErrorCode::InternalError, format!("serialize: {e}")))?;
    {
        let mut sink = conn.sink.lock().await;
        if let Err(e) =
            futures_util::SinkExt::send(&mut *sink, axum::extract::ws::Message::Text(json.into()))
                .await
        {
            return Err(ApiError::new(
                ErrorCode::DeviceOffline,
                format!("send ReadStream: {e}"),
            ));
        }
    }
    drop(conn);

    let mime = detect_mime_from_extension(&path);
    let body_stream = build_body_stream(rx, guard);

    let mut resp = Response::new(Body::from_stream(body_stream));
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        mime.parse()
            .unwrap_or_else(|_| "application/octet-stream".parse().expect("static")),
    );
    resp.headers_mut()
        .insert("X-Content-Type-Options", "nosniff".parse().expect("static"));
    Ok(resp)
}

/// Convert an mpsc of `StreamFrame`s into a `Stream<Item = Result<Vec<u8>, io::Error>>`.
/// The `SlotGuard` is carried in the unfold state so it drops exactly when
/// the body stream is dropped.
fn build_body_stream(
    rx: mpsc::Receiver<StreamFrame>,
    guard: SlotGuard,
) -> impl Stream<Item = Result<Vec<u8>, std::io::Error>> + Send + 'static {
    struct S {
        rx: mpsc::Receiver<StreamFrame>,
        _guard: SlotGuard,
        done: bool,
    }
    let init = S {
        rx,
        _guard: guard,
        done: false,
    };
    unfold(init, |mut s| async move {
        if s.done {
            return None;
        }
        match tokio::time::timeout(STREAM_IDLE_TIMEOUT, s.rx.recv()).await {
            Ok(Some(StreamFrame::Chunk(data))) => Some((Ok(data), s)),
            Ok(Some(StreamFrame::End)) | Ok(None) => None,
            Ok(Some(StreamFrame::Error(msg))) => {
                s.done = true;
                Some((Err(std::io::Error::other(msg)), s))
            }
            Err(_elapsed) => {
                s.done = true;
                Some((
                    Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "device stream idle timeout",
                    )),
                    s,
                ))
            }
        }
    })
}

/// Route a freshly arrived `StreamChunk` / `StreamEnd` / `StreamError` to
/// its waiting HTTP handler. Called from the WS message loop. If no slot
/// exists (late frame after handler timeout / disconnect) the frame is
/// silently dropped.
pub fn dispatch_frame(state: &AppState, device_key: &str, request_id: &str, frame: StreamFrame) {
    let Some(slots) = state.streams.get(device_key) else {
        return;
    };
    let Some(sender) = slots.get(request_id) else {
        return;
    };
    // try_send: if the receiver is full we drop the frame rather than
    // block the WS loop. A well-behaved client respects the implicit
    // backpressure from the WS sink's buffer.
    let _ = sender.try_send(frame);
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn dispatch_frame_with_no_slot_is_a_noop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = AppState::test_minimal(tmp.path());
        // No panic, no side effect.
        dispatch_frame(
            &state,
            "alice:box",
            "missing-req-id",
            StreamFrame::Chunk(vec![1, 2, 3]),
        );
    }

    #[tokio::test]
    async fn dispatch_frame_delivers_to_registered_slot() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = AppState::test_minimal(tmp.path());

        let device_key = "alice:box".to_string();
        let request_id = "req-1".to_string();
        let (tx, mut rx) = mpsc::channel::<StreamFrame>(4);
        state
            .streams
            .entry(device_key.clone())
            .or_default()
            .insert(request_id.clone(), tx);

        dispatch_frame(
            &state,
            &device_key,
            &request_id,
            StreamFrame::Chunk(vec![7, 8, 9]),
        );
        dispatch_frame(&state, &device_key, &request_id, StreamFrame::End);

        match rx.recv().await.expect("chunk") {
            StreamFrame::Chunk(d) => assert_eq!(d, vec![7, 8, 9]),
            f => panic!("expected Chunk, got {f:?}"),
        }
        match rx.recv().await.expect("end") {
            StreamFrame::End => {}
            f => panic!("expected End, got {f:?}"),
        }
    }

    #[tokio::test]
    async fn body_stream_yields_chunks_then_terminates_on_end() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = AppState::test_minimal(tmp.path());
        let device_key = "alice:box".to_string();
        let request_id = "req-end".to_string();
        let (tx, rx) = mpsc::channel::<StreamFrame>(4);
        state
            .streams
            .entry(device_key.clone())
            .or_default()
            .insert(request_id.clone(), tx.clone());
        let guard = SlotGuard {
            state: state.clone(),
            device_key: device_key.clone(),
            request_id: request_id.clone(),
        };

        // Feed frames.
        tx.send(StreamFrame::Chunk(b"hello".to_vec()))
            .await
            .unwrap();
        tx.send(StreamFrame::Chunk(b" world".to_vec()))
            .await
            .unwrap();
        tx.send(StreamFrame::End).await.unwrap();
        drop(tx); // mirror real-world: producer side closes after End

        let stream = build_body_stream(rx, guard);
        tokio::pin!(stream);
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            out.extend_from_slice(&item.expect("no error"));
        }
        assert_eq!(out, b"hello world");

        // Slot cleanup should have happened via guard drop.
        assert!(
            state
                .streams
                .get(&device_key)
                .map(|m| !m.contains_key(&request_id))
                .unwrap_or(true)
        );
    }

    #[tokio::test]
    async fn body_stream_surfaces_stream_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = AppState::test_minimal(tmp.path());
        let device_key = "alice:box".to_string();
        let request_id = "req-err".to_string();
        let (tx, rx) = mpsc::channel::<StreamFrame>(4);
        state
            .streams
            .entry(device_key.clone())
            .or_default()
            .insert(request_id.clone(), tx.clone());
        let guard = SlotGuard {
            state: state.clone(),
            device_key,
            request_id,
        };

        tx.send(StreamFrame::Chunk(b"partial".to_vec()))
            .await
            .unwrap();
        tx.send(StreamFrame::Error("disk read failed".into()))
            .await
            .unwrap();
        drop(tx);

        let stream = build_body_stream(rx, guard);
        tokio::pin!(stream);
        let first = stream.next().await.expect("chunk").expect("ok");
        assert_eq!(first, b"partial");
        let err = stream.next().await.expect("err").expect_err("err expected");
        assert!(err.to_string().contains("disk read failed"));
        // Stream terminates after an error.
        assert!(stream.next().await.is_none());
    }
}
