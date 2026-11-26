//! In-flight request registry, read by the N2 watchdog.
//!
//! N2 (server goes HTTP-dead while the process lives) needs to know, at the
//! moment of death, which requests are still open and in which phase:
//!
//! - `AwaitingHeaders` — the handler has not produced a response yet. One that
//!   is old means a handler is wedged (a lock or DB call that never returns).
//! - `StreamingBody` — headers are out and the body is still being sent. Long
//!   ones are normal (SSE, video); a pile of them is the S1 suspicion (stalled
//!   streams pinning blocking-pool threads).
//!
//! The registry guard lives inside the response body, so a request stays
//! visible until its body is finished or dropped, not just until the handler
//! returns. Only the path is recorded: query strings can carry tokens.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use http_body::{Frame, SizeHint};

/// A handler slower than this to produce headers is logged when it finishes.
pub const SLOW_HEADERS: Duration = Duration::from_secs(10);

/// Longest path recorded per request (paths are attacker-controlled).
const MAX_PATH_LEN: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    AwaitingHeaders,
    StreamingBody,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::AwaitingHeaders => "awaiting_headers",
            Phase::StreamingBody => "streaming_body",
        }
    }
}

/// One open request, as seen by a snapshot.
#[derive(Clone, Debug)]
pub struct InFlight {
    pub method: String,
    pub path: String,
    pub age: Duration,
    pub phase: Phase,
}

struct Entry {
    method: String,
    path: String,
    started: Instant,
    phase: Phase,
}

#[derive(Default)]
pub struct Registry {
    next_id: AtomicU64,
    entries: Mutex<HashMap<u64, Entry>>,
}

impl Registry {
    /// The registry the server's tracking layer and watchdog share.
    pub fn global() -> Arc<Registry> {
        static GLOBAL: OnceLock<Arc<Registry>> = OnceLock::new();
        Arc::clone(GLOBAL.get_or_init(|| Arc::new(Registry::default())))
    }

    /// The map is only touched in short, non-async critical sections. A
    /// poisoned lock still holds a consistent map (every mutation is a single
    /// insert/remove/field write), so recover it instead of losing the data.
    fn lock(&self) -> MutexGuard<'_, HashMap<u64, Entry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Register a request; it stays registered until the guard drops.
    pub fn begin(self: &Arc<Self>, method: &str, path: &str) -> Guard {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let path = truncate(path, MAX_PATH_LEN);
        self.lock().insert(
            id,
            Entry {
                method: method.to_owned(),
                path,
                started: Instant::now(),
                phase: Phase::AwaitingHeaders,
            },
        );
        Guard {
            registry: Arc::clone(self),
            id,
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Every open request, oldest first.
    pub fn snapshot(&self, now: Instant) -> Vec<InFlight> {
        let mut out: Vec<InFlight> = self
            .lock()
            .values()
            .map(|e| InFlight {
                method: e.method.clone(),
                path: e.path.clone(),
                age: now.saturating_duration_since(e.started),
                phase: e.phase,
            })
            .collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.age));
        out
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Removes its request from the registry on drop — whether the body finished,
/// the client disconnected, or the handler future was cancelled.
pub struct Guard {
    registry: Arc<Registry>,
    id: u64,
}

impl Guard {
    pub fn headers_sent(&self) {
        if let Some(e) = self.registry.lock().get_mut(&self.id) {
            e.phase = Phase::StreamingBody;
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.registry.lock().remove(&self.id);
    }
}

/// Response body that carries the request's guard. Forwards everything —
/// `size_hint` in particular, or every response would lose `Content-Length`
/// (and with it `Accept-Ranges` seeking, cf. B4).
pub struct TrackedBody {
    inner: Body,
    _guard: Guard,
}

impl http_body::Body for TrackedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.get_mut().inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// Middleware: register the request, warn on slow headers, and hand the guard
/// to the body. Install with `from_fn_with_state(Registry::global(), track)`.
pub async fn track(State(registry): State<Arc<Registry>>, req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_owned();
    let path = req.uri().path().to_owned();
    let guard = registry.begin(&method, &path);
    let started = Instant::now();

    let response = next.run(req).await;

    let waited = started.elapsed();
    if waited >= SLOW_HEADERS {
        tracing::warn!(
            method = %method,
            path = %truncate(&path, MAX_PATH_LEN),
            status = response.status().as_u16(),
            secs = waited.as_secs_f64(),
            "Slow request: response headers took longer than {}s",
            SLOW_HEADERS.as_secs()
        );
    }
    guard.headers_sent();
    response.map(|inner| {
        Body::new(TrackedBody {
            inner,
            _guard: guard,
        })
    })
}

// ── Chunked-decrypt stream gauge (N2 suspicion S1) ───────────────────────

static DECRYPT_STREAMS: AtomicUsize = AtomicUsize::new(0);

/// Live chunked-decrypt threads (`photos/serve.rs`). Each one holds a
/// blocking-pool thread until its client has read the whole body or hung up.
pub fn decrypt_streams() -> usize {
    DECRYPT_STREAMS.load(Ordering::Relaxed)
}

/// Counts one decrypt thread for as long as it is alive.
pub struct DecryptStreamGauge(());

impl DecryptStreamGauge {
    pub fn enter() -> Self {
        DECRYPT_STREAMS.fetch_add(1, Ordering::Relaxed);
        DecryptStreamGauge(())
    }
}

impl Drop for DecryptStreamGauge {
    fn drop(&mut self) {
        DECRYPT_STREAMS.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use http_body::Body as _;
    use tower::ServiceExt;

    fn app(registry: Arc<Registry>) -> Router {
        Router::new()
            .route("/hello", get(|| async { "hello" }))
            .fallback(|| async { "fallback" })
            .layer(axum::middleware::from_fn_with_state(registry, track))
    }

    #[test]
    fn guard_drop_removes_entry_and_snapshot_is_oldest_first() {
        let reg = Arc::new(Registry::default());
        let old = reg.begin("GET", "/old");
        std::thread::sleep(Duration::from_millis(5));
        let new = reg.begin("POST", "/new");
        new.headers_sent();

        let snap = reg.snapshot(Instant::now());
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].path, "/old");
        assert_eq!(snap[0].phase, Phase::AwaitingHeaders);
        assert_eq!(snap[1].path, "/new");
        assert_eq!(snap[1].phase, Phase::StreamingBody);

        drop(old);
        assert_eq!(reg.len(), 1);
        drop(new);
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn long_paths_are_truncated_on_a_char_boundary() {
        let reg = Arc::new(Registry::default());
        let long = format!("/{}", "é".repeat(200));
        let _g = reg.begin("GET", &long);
        let snap = reg.snapshot(Instant::now());
        assert!(snap[0].path.len() <= MAX_PATH_LEN + '…'.len_utf8());
        assert!(snap[0].path.ends_with('…'));
    }

    #[tokio::test]
    async fn request_stays_registered_until_its_body_is_dropped_and_keeps_size_hint() {
        let reg = Arc::new(Registry::default());
        let resp = app(Arc::clone(&reg))
            .oneshot(
                Request::builder()
                    .uri("/hello?token=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Content-Length survives the wrapper.
        assert_eq!(resp.body().size_hint().exact(), Some(5));

        let snap = reg.snapshot(Instant::now());
        assert_eq!(snap.len(), 1, "body not finished ⇒ still in flight");
        assert_eq!(snap[0].path, "/hello", "query string must not be recorded");
        assert_eq!(snap[0].phase, Phase::StreamingBody);

        drop(resp);
        assert_eq!(reg.len(), 0);
    }

    #[tokio::test]
    async fn fallback_routes_are_tracked_too() {
        let reg = Arc::new(Registry::default());
        let resp = app(Arc::clone(&reg))
            .oneshot(
                Request::builder()
                    .uri("/no/such/route")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reg.snapshot(Instant::now())[0].path, "/no/such/route");
        drop(resp);
    }

    #[test]
    fn decrypt_gauge_counts_live_guards() {
        let before = decrypt_streams();
        let a = DecryptStreamGauge::enter();
        let b = DecryptStreamGauge::enter();
        assert!(decrypt_streams() >= before + 2);
        drop(a);
        drop(b);
        // Other tests may hold gauges concurrently; ours are gone.
        assert!(decrypt_streams() <= before + 2);
    }
}
