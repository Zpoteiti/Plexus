//! Handler for `ServerToClient::ReadStream` — FR1b.
//!
//! The server initiates a file stream via `ReadStream { request_id, path }`.
//! We chunk-read the file (32 KiB chunks) and emit
//! `ClientToServer::StreamChunk` frames, terminating with `StreamEnd` or
//! `StreamError`.
//!
//! Concurrency is bounded to 4 in-flight streams per client. The 5th and
//! beyond are rejected synchronously with `StreamError { error: "too many
//! concurrent streams" }` so a buggy server cannot DoS the device's disk.

use crate::config::ClientConfig;
use crate::connection::{WsSink, send_message};
use crate::tools::helpers::sanitize_path;
use plexus_common::protocol::ClientToServer;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::sync::{Mutex, RwLock, Semaphore, TryAcquireError};
use tracing::warn;

/// Chunk size for streamed reads. WS frames are bounded — do not raise.
pub const CHUNK_SIZE: usize = 32 * 1024;

/// Max concurrent in-flight streams per client.
pub const MAX_CONCURRENT_STREAMS: usize = 4;

/// Abstraction over the outbound frame sink so the handler is unit-testable
/// without a real WebSocket. The real impl forwards over the shared
/// `Arc<Mutex<WsSink>>`; tests implement it with a `Vec<ClientToServer>`
/// capture. Uses `Pin<Box<dyn Future>>` (the same pattern `Tool` uses in
/// `tools/mod.rs`) to avoid pulling in `async_trait` for a single call site.
pub trait FrameSink: Send + Sync {
    fn send<'a>(&'a self, msg: ClientToServer) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// Real sink — forwards to the WebSocket. `warn!`s on send failures; the
/// session loop will detect dead connections via heartbeat regardless.
pub struct WsFrameSink {
    pub sink: Arc<Mutex<WsSink>>,
}

impl FrameSink for WsFrameSink {
    fn send<'a>(&'a self, msg: ClientToServer) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let mut guard = self.sink.lock().await;
            if let Err(e) = send_message(&mut guard, &msg).await {
                warn!("send stream frame failed: {e}");
            }
        })
    }
}

/// Shared concurrency gate for streaming requests. Wrap in `Arc` so the
/// main session can hand clones to spawned handlers.
pub fn new_stream_semaphore() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(MAX_CONCURRENT_STREAMS))
}

/// Handle a `ReadStream` request end-to-end. Caller passes a shared
/// `Semaphore` from `new_stream_semaphore`. The function rejects
/// synchronously when the semaphore has no permits available.
pub async fn handle(
    sink: Arc<dyn FrameSink>,
    config: Arc<RwLock<ClientConfig>>,
    semaphore: Arc<Semaphore>,
    request_id: String,
    path: String,
) {
    // Bounded concurrency gate — synchronous rejection when saturated.
    let _permit = match semaphore.try_acquire_owned() {
        Ok(p) => p,
        Err(TryAcquireError::NoPermits) => {
            sink.send(ClientToServer::StreamError {
                request_id,
                error: "too many concurrent streams".to_string(),
            })
            .await;
            return;
        }
        Err(TryAcquireError::Closed) => {
            sink.send(ClientToServer::StreamError {
                request_id,
                error: "stream semaphore closed".to_string(),
            })
            .await;
            return;
        }
    };

    // Snapshot config so we don't hold the read lock for the duration of the
    // stream (which could block ConfigUpdate writers).
    let cfg_snapshot = { config.read().await.clone() };

    // Path validation — reuses the `sanitize_path` helper which already
    // encodes the sandbox vs unrestricted rule. `write=false` because we are
    // only reading.
    let resolved = match sanitize_path(&path, &cfg_snapshot, false) {
        Ok(p) => p,
        Err(e) => {
            // `sanitize_path` returns a formatted tool_error for sandbox
            // escapes; for the stream protocol we prefer a short, stable
            // identifier so the server-side consumer doesn't have to
            // substring-match.
            let err = if e.contains("outside workspace") {
                "path outside workspace".to_string()
            } else {
                e
            };
            sink.send(ClientToServer::StreamError {
                request_id,
                error: err,
            })
            .await;
            return;
        }
    };

    let mut file = match tokio::fs::File::open(&resolved).await {
        Ok(f) => f,
        Err(e) => {
            sink.send(ClientToServer::StreamError {
                request_id,
                error: format!("open failed: {e}"),
            })
            .await;
            return;
        }
    };

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut offset: u64 = 0;
    loop {
        match file.read(&mut buf).await {
            Ok(0) => {
                sink.send(ClientToServer::StreamEnd {
                    request_id,
                    total_size: offset,
                })
                .await;
                return;
            }
            Ok(n) => {
                sink.send(ClientToServer::StreamChunk {
                    request_id: request_id.clone(),
                    data: buf[..n].to_vec(),
                    offset,
                })
                .await;
                offset += n as u64;
            }
            Err(e) => {
                sink.send(ClientToServer::StreamError {
                    request_id,
                    error: format!("read failed: {e}"),
                })
                .await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plexus_common::protocol::FsPolicy;
    use std::path::PathBuf;

    /// Test sink that captures emitted frames in order.
    struct CapturingSink {
        frames: Mutex<Vec<ClientToServer>>,
    }

    impl CapturingSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                frames: Mutex::new(Vec::new()),
            })
        }

        async fn frames(&self) -> Vec<ClientToServer> {
            self.frames.lock().await.clone()
        }
    }

    impl FrameSink for CapturingSink {
        fn send<'a>(
            &'a self,
            msg: ClientToServer,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                self.frames.lock().await.push(msg);
            })
        }
    }

    fn cfg(workspace: PathBuf, fs_policy: FsPolicy) -> Arc<RwLock<ClientConfig>> {
        Arc::new(RwLock::new(ClientConfig {
            workspace,
            fs_policy,
            shell_timeout_max: 60,
            ssrf_whitelist: vec![],
            mcp_servers: vec![],
        }))
    }

    fn is_chunk(f: &ClientToServer) -> bool {
        matches!(f, ClientToServer::StreamChunk { .. })
    }

    fn is_end(f: &ClientToServer) -> bool {
        matches!(f, ClientToServer::StreamEnd { .. })
    }

    fn is_error(f: &ClientToServer) -> bool {
        matches!(f, ClientToServer::StreamError { .. })
    }

    #[tokio::test]
    async fn path_inside_workspace_emits_chunks_and_end() {
        let d = tempfile::tempdir().unwrap();
        // 32 KiB + 100 bytes → expect 2 chunks + 1 end.
        let payload: Vec<u8> = (0..(CHUNK_SIZE as u32 + 100))
            .map(|i| (i & 0xFF) as u8)
            .collect();
        let p = d.path().join("f.bin");
        std::fs::write(&p, &payload).unwrap();

        let sink = CapturingSink::new();
        let cast: Arc<dyn FrameSink> = sink.clone();
        handle(
            cast,
            cfg(d.path().to_path_buf(), FsPolicy::Sandbox),
            new_stream_semaphore(),
            "req-1".into(),
            p.to_string_lossy().to_string(),
        )
        .await;

        let frames = sink.frames().await;
        let chunks: Vec<_> = frames.iter().filter(|f| is_chunk(f)).collect();
        assert_eq!(chunks.len(), 2, "expected 2 chunks, got {frames:?}");
        assert!(is_end(frames.last().unwrap()));

        // Offsets must be monotonic starting at 0.
        if let ClientToServer::StreamChunk { offset, .. } = chunks[0] {
            assert_eq!(*offset, 0);
        }
        if let ClientToServer::StreamChunk { offset, .. } = chunks[1] {
            assert_eq!(*offset, CHUNK_SIZE as u64);
        }

        // total_size equals payload length.
        if let ClientToServer::StreamEnd { total_size, .. } = frames.last().unwrap() {
            assert_eq!(*total_size, payload.len() as u64);
        }

        // Reassembled bytes equal the original.
        let mut rebuilt = Vec::<u8>::new();
        for f in &frames {
            if let ClientToServer::StreamChunk { data, .. } = f {
                rebuilt.extend_from_slice(data);
            }
        }
        assert_eq!(rebuilt, payload);
    }

    #[tokio::test]
    async fn path_outside_workspace_sandbox_rejected() {
        let d = tempfile::tempdir().unwrap(); // workspace
        let other = tempfile::tempdir().unwrap(); // outside
        let victim = other.path().join("secret.txt");
        std::fs::write(&victim, b"topsecret").unwrap();

        let sink = CapturingSink::new();
        let cast: Arc<dyn FrameSink> = sink.clone();
        handle(
            cast,
            cfg(d.path().to_path_buf(), FsPolicy::Sandbox),
            new_stream_semaphore(),
            "req-2".into(),
            victim.to_string_lossy().to_string(),
        )
        .await;

        let frames = sink.frames().await;
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            ClientToServer::StreamError { error, .. } => {
                assert_eq!(error, "path outside workspace");
            }
            other => panic!("expected StreamError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn path_outside_workspace_unrestricted_allowed() {
        let d = tempfile::tempdir().unwrap(); // workspace
        let other = tempfile::tempdir().unwrap(); // outside
        let target = other.path().join("ok.txt");
        std::fs::write(&target, b"hello").unwrap();

        let sink = CapturingSink::new();
        let cast: Arc<dyn FrameSink> = sink.clone();
        handle(
            cast,
            cfg(d.path().to_path_buf(), FsPolicy::Unrestricted),
            new_stream_semaphore(),
            "req-3".into(),
            target.to_string_lossy().to_string(),
        )
        .await;

        let frames = sink.frames().await;
        assert!(frames.iter().any(is_chunk));
        assert!(is_end(frames.last().unwrap()));
    }

    #[tokio::test]
    async fn missing_file_emits_stream_error() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("no_such_file.bin");

        let sink = CapturingSink::new();
        let cast: Arc<dyn FrameSink> = sink.clone();
        handle(
            cast,
            cfg(d.path().to_path_buf(), FsPolicy::Unrestricted),
            new_stream_semaphore(),
            "req-4".into(),
            missing.to_string_lossy().to_string(),
        )
        .await;

        let frames = sink.frames().await;
        assert_eq!(frames.len(), 1);
        assert!(is_error(&frames[0]));
    }

    #[tokio::test]
    async fn concurrency_cap_rejects_fifth_request() {
        // A "slow" sink parks on a Notify so the first 4 handlers stay
        // in-flight (each holding a permit) long enough for the 5th to
        // observe saturation.
        use tokio::sync::Notify;

        struct SlowSink {
            release: Arc<Notify>,
        }
        impl FrameSink for SlowSink {
            fn send<'a>(
                &'a self,
                _msg: ClientToServer,
            ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
                let release = self.release.clone();
                Box::pin(async move {
                    release.notified().await;
                })
            }
        }

        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f.bin");
        std::fs::write(&p, b"hello").unwrap();

        let semaphore = new_stream_semaphore();
        let cfg = cfg(d.path().to_path_buf(), FsPolicy::Unrestricted);

        let release = Arc::new(Notify::new());
        let mut handles = Vec::new();
        for i in 0..MAX_CONCURRENT_STREAMS {
            let slow: Arc<dyn FrameSink> = Arc::new(SlowSink {
                release: release.clone(),
            });
            let cfg = cfg.clone();
            let sem = semaphore.clone();
            let path_str = p.to_string_lossy().to_string();
            handles.push(tokio::spawn(async move {
                handle(slow, cfg, sem, format!("slow-{i}"), path_str).await;
            }));
        }

        // Wait until all permits are held. Each handler grabs its permit
        // before its first `send`, so a short poll is sufficient.
        for _ in 0..50 {
            if semaphore.available_permits() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            semaphore.available_permits(),
            0,
            "expected all permits held"
        );

        // 5th request must be rejected immediately with StreamError.
        let sink5 = CapturingSink::new();
        let cast: Arc<dyn FrameSink> = sink5.clone();
        handle(
            cast,
            cfg.clone(),
            semaphore.clone(),
            "rejected".into(),
            p.to_string_lossy().to_string(),
        )
        .await;

        let f = sink5.frames().await;
        assert_eq!(f.len(), 1);
        match &f[0] {
            ClientToServer::StreamError { error, .. } => {
                assert_eq!(error, "too many concurrent streams");
            }
            other => panic!("expected StreamError, got {other:?}"),
        }

        // Release parked tasks and clean up so nothing leaks.
        for _ in 0..(MAX_CONCURRENT_STREAMS * 4) {
            release.notify_one();
        }
        for h in handles {
            h.abort();
        }
    }
}
