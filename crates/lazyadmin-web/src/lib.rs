#![forbid(unsafe_code)]

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive};
use axum::{
    Router,
    extract::{Path, Request, State},
    http::{StatusCode, header::HOST},
    response::{Html, IntoResponse, Json, Response, Sse},
    routing::get,
};
use futures::{Stream, StreamExt};
use lazyadmin_core::{
    config::Config,
    model::{DiscoveryEvent, Snapshot},
};
use serde::Serialize;
use tokio::sync::{RwLock, broadcast};
use tokio_stream::wrappers::BroadcastStream;
use tower_http::{compression::CompressionLayer, trace::TraceLayer};

#[derive(Clone, Debug)]
pub struct WebOptions {
    pub bind: IpAddr,
    pub port: u16,
    pub config_path: Option<PathBuf>,
    pub refresh_interval: Duration,
}

impl Default for WebOptions {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 7749,
            config_path: None,
            refresh_interval: Duration::from_millis(2_000),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ServerInfo {
    pub url: String,
    pub addr: SocketAddr,
}

#[derive(Clone)]
struct AppState {
    config_path: Option<PathBuf>,
    started_at: Instant,
    last_snapshot: Arc<RwLock<Option<CachedSnapshot>>>,
    events: broadcast::Sender<DiscoveryEvent>,
}

#[derive(Clone)]
struct CachedSnapshot {
    snapshot: Snapshot,
    refreshed_at: Instant,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    version: &'static str,
    bind_policy: &'static str,
    uptime_ms: u128,
    last_snapshot_age_ms: Option<u128>,
    last_snapshot_status: &'static str,
}

#[derive(Serialize)]
struct OverviewResponse {
    listeners: usize,
    public_listeners: usize,
    workloads: usize,
    processes: usize,
    managers: usize,
    projects: usize,
    tracked_runs: usize,
    warnings: usize,
    events_dropped: u64,
}

#[derive(Serialize)]
struct ApiError<'a> {
    code: &'a str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

/// Bind the configured address and prepare the server. Returns the resolved
/// `ServerInfo` plus a `JoinHandle` running the HTTP server. The caller is
/// responsible for awaiting the handle to keep the server alive.
///
/// This is the entry point used by both the `lazyadmin web` CLI command and
/// integration tests. The `bind_for_test` alias is kept for backwards
/// compatibility with the original PLAN-14 milestone wiring.
pub async fn bind(
    options: WebOptions,
) -> anyhow::Result<(ServerInfo, tokio::task::JoinHandle<anyhow::Result<()>>)> {
    ensure_loopback(options.bind)?;
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(options.bind, options.port)).await?;
    let local_addr = listener.local_addr()?;
    let state = AppState::new(options.config_path.clone(), options.refresh_interval).await?;
    let app = app(state);
    let url = format!("http://{}:{}/", local_addr.ip(), local_addr.port());
    tracing::info!(%url, "lazyadmin web server listening");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .map_err(Into::into)
    });
    Ok((
        ServerInfo {
            url,
            addr: local_addr,
        },
        handle,
    ))
}

/// Bind and run the server until it terminates. Convenience wrapper around
/// [`bind`] for callers that do not need access to `ServerInfo` mid-run.
pub async fn serve(options: WebOptions) -> anyhow::Result<ServerInfo> {
    let (info, handle) = bind(options).await?;
    handle.await??;
    Ok(info)
}

#[deprecated(note = "use `bind`; retained for compatibility with PLAN-14 milestone wiring")]
pub async fn bind_for_test(
    options: WebOptions,
) -> anyhow::Result<(ServerInfo, tokio::task::JoinHandle<anyhow::Result<()>>)> {
    bind(options).await
}

fn ensure_loopback(bind: IpAddr) -> anyhow::Result<()> {
    if bind.is_loopback() {
        Ok(())
    } else {
        anyhow::bail!("lazyadmin web is local-only in v1; refusing non-loopback bind {bind}")
    }
}

fn app(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/snapshot", get(snapshot))
        .route("/doctor", get(doctor))
        .route("/events", get(events))
        .route("/views/overview", get(overview))
        .route("/entities/:kind/:id", get(entity))
        .layer(middleware::from_fn(local_origin_guard));
    Router::new()
        .route("/", get(index))
        .nest("/api", api)
        .with_state(state)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}

/// Reject API requests whose `Host` header is not loopback. Even on a
/// loopback bind this prevents DNS rebinding attacks where a remote page
/// resolves an attacker-controlled hostname to `127.0.0.1` and tries to
/// drive the read-only API from the user's browser.
async fn local_origin_guard(req: Request, next: Next) -> Response {
    if let Some(host) = req.headers().get(HOST).and_then(|v| v.to_str().ok()) {
        if !is_local_host(host) {
            return api_error(
                StatusCode::FORBIDDEN,
                "NON_LOCAL_HOST",
                format!("refusing API request with non-local Host header {host:?}"),
                None,
            );
        }
    }
    next.run(req).await
}

fn is_local_host(host: &str) -> bool {
    // Strip optional port. Bracketed IPv6 hosts (`[::1]:port`) need special handling,
    // and bare IPv6 hosts contain colons themselves so we must avoid splitting on the
    // first colon for those.
    let bare = if let Some(stripped) = host.strip_prefix('[') {
        match stripped.find(']') {
            Some(end) => &stripped[..end],
            None => host,
        }
    } else if host.matches(':').count() > 1 {
        // Likely a bare IPv6 literal like `::1` (potentially without a port).
        host
    } else {
        host.split(':').next().unwrap_or(host)
    };
    matches!(bare, "localhost") || bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

impl AppState {
    async fn new(config_path: Option<PathBuf>, refresh_interval: Duration) -> anyhow::Result<Self> {
        let (events, _) = broadcast::channel(128);
        let state = Self {
            config_path,
            started_at: Instant::now(),
            last_snapshot: Arc::new(RwLock::new(None)),
            events,
        };
        state.refresh_snapshot().await?;
        state.spawn_refresh(refresh_interval);
        Ok(state)
    }

    async fn refresh_snapshot(&self) -> anyhow::Result<Snapshot> {
        let snapshot = lazyadmin_runtime::build_snapshot(self.config_path.as_deref()).await?;
        *self.last_snapshot.write().await = Some(CachedSnapshot {
            snapshot: snapshot.clone(),
            refreshed_at: Instant::now(),
        });
        Ok(snapshot)
    }

    fn spawn_refresh(&self, refresh_interval: Duration) {
        let state = self.clone();
        tokio::spawn(async move {
            let cfg = Config::load(state.config_path.as_deref()).unwrap_or_default();
            let streams = lazyadmin_runtime::event_streams_for_config(&cfg).await;
            let mut events = futures::stream::select_all(streams);
            let mut interval = tokio::time::interval(refresh_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => { let _ = state.refresh_snapshot().await; }
                    event = events.next(), if !events.is_empty() => {
                        if let Some(event) = event {
                            let _ = state.events.send(event);
                            let _ = state.refresh_snapshot().await;
                        }
                    }
                }
            }
        });
    }

    async fn snapshot(&self) -> anyhow::Result<Snapshot> {
        if let Some(cached) = self.last_snapshot.read().await.clone() {
            Ok(cached.snapshot)
        } else {
            self.refresh_snapshot().await
        }
    }
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let cached = state.last_snapshot.read().await;
    Json(HealthResponse {
        ok: cached.is_some(),
        version: env!("CARGO_PKG_VERSION"),
        bind_policy: "loopback-only",
        uptime_ms: state.started_at.elapsed().as_millis(),
        last_snapshot_age_ms: cached
            .as_ref()
            .map(|c| c.refreshed_at.elapsed().as_millis()),
        last_snapshot_status: if cached.is_some() { "ok" } else { "missing" },
    })
}

async fn snapshot(State(state): State<AppState>) -> impl IntoResponse {
    json_result(state.snapshot().await)
}

async fn doctor(State(state): State<AppState>) -> impl IntoResponse {
    match state.snapshot().await {
        Ok(snapshot) => Json(lazyadmin_runtime::view_model::build_doctor_groups(
            &snapshot,
        ))
        .into_response(),
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SNAPSHOT_FAILED",
            e.to_string(),
            None,
        ),
    }
}

async fn overview(State(state): State<AppState>) -> impl IntoResponse {
    match state.snapshot().await {
        Ok(s) => {
            let public_listeners = s
                .listeners
                .iter()
                .filter(|l| {
                    !matches!(
                        l.exposure,
                        lazyadmin_core::model::Exposure::Loopback
                            | lazyadmin_core::model::Exposure::UnixLocal
                    )
                })
                .count();
            Json(OverviewResponse {
                listeners: s.listeners.len(),
                public_listeners,
                workloads: s.workloads.len(),
                processes: s.processes.len(),
                managers: s.managers.len(),
                projects: s.projects.len(),
                tracked_runs: s.tracked_runs.len(),
                warnings: s.warnings.len(),
                events_dropped: s
                    .metadata
                    .as_ref()
                    .and_then(|m| m.events_dropped)
                    .unwrap_or(0),
            })
            .into_response()
        }
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SNAPSHOT_FAILED",
            e.to_string(),
            None,
        ),
    }
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let stream = BroadcastStream::new(state.events.subscribe()).filter_map(|event| async move {
        match event {
            Ok(event) => Some(Ok(Event::default()
                .event("discovery")
                .json_data(event)
                .unwrap_or_else(|_| {
                    Event::default().event("error").data("serialization failed")
                }))),
            Err(_) => None,
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn entity(
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, String)>,
) -> impl IntoResponse {
    let Ok(snapshot) = state.snapshot().await else {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SNAPSHOT_FAILED",
            "snapshot unavailable",
            None,
        );
    };
    let value = match kind.as_str() {
        "listener" | "listeners" => snapshot
            .listeners
            .iter()
            .find(|l| l.id.to_string() == id)
            .map(serde_json::to_value),
        "process" | "processes" => snapshot
            .processes
            .iter()
            .find(|p| {
                serde_json::to_string(&p.key).ok().is_some_and(|k| k == id)
                    || p.pid.to_string() == id
            })
            .map(serde_json::to_value),
        "workload" | "workloads" => snapshot
            .workloads
            .iter()
            .find(|w| w.id.to_string() == id)
            .map(serde_json::to_value),
        "manager" | "managers" => snapshot
            .managers
            .iter()
            .find(|m| m.id.to_string() == id)
            .map(serde_json::to_value),
        "project" | "projects" => snapshot
            .projects
            .iter()
            .find(|p| p.id.to_string() == id)
            .map(serde_json::to_value),
        "run" | "runs" => snapshot
            .tracked_runs
            .iter()
            .find(|r| r.id.to_string() == id)
            .map(serde_json::to_value),
        _ => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "INVALID_ENTITY_KIND",
                format!("unknown entity kind {kind}"),
                None,
            );
        }
    };
    match value.transpose() {
        Ok(Some(value)) => Json(value).into_response(),
        Ok(None) => api_error(
            StatusCode::NOT_FOUND,
            "ENTITY_NOT_FOUND",
            format!("{kind}/{id} was not found"),
            None,
        ),
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SERIALIZE_FAILED",
            e.to_string(),
            None,
        ),
    }
}

fn json_result<T: Serialize>(result: anyhow::Result<T>) -> axum::response::Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SNAPSHOT_FAILED",
            e.to_string(),
            None,
        ),
    }
}

fn api_error(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> axum::response::Response {
    (
        status,
        Json(ApiError {
            code,
            message: message.into(),
            details,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_loopback() {
        assert!(ensure_loopback("0.0.0.0".parse().unwrap()).is_err());
        assert!(ensure_loopback("127.0.0.1".parse().unwrap()).is_ok());
        assert!(ensure_loopback("::1".parse().unwrap()).is_ok());
    }

    #[test]
    fn classifies_local_hosts() {
        for host in [
            "localhost",
            "localhost:7749",
            "127.0.0.1",
            "127.0.0.1:8080",
            "::1",
            "[::1]",
            "[::1]:7749",
        ] {
            assert!(is_local_host(host), "expected {host} to be local");
        }
        for host in ["example.com", "example.com:7749", "192.168.1.5"] {
            assert!(!is_local_host(host), "expected {host} to be non-local");
        }
    }

    async fn build_test_app() -> Router {
        let state = AppState::new(None, Duration::from_secs(60))
            .await
            .expect("state builds with default config");
        app(state)
    }

    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        use http_body_util::BodyExt;
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).expect("valid json")
    }

    fn local_request(uri: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri(uri)
            .header("host", "127.0.0.1")
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn health_and_overview_round_trip() {
        use tower::ServiceExt;
        let app = build_test_app().await;
        let health = app
            .clone()
            .oneshot(local_request("/api/health"))
            .await
            .expect("health response");
        assert_eq!(health.status(), StatusCode::OK);
        let body = json_body(health).await;
        assert_eq!(body["bind_policy"], "loopback-only");
        let overview = app
            .clone()
            .oneshot(local_request("/api/views/overview"))
            .await
            .expect("overview response");
        assert_eq!(overview.status(), StatusCode::OK);
        let body = json_body(overview).await;
        for key in [
            "listeners",
            "public_listeners",
            "workloads",
            "processes",
            "managers",
            "projects",
            "tracked_runs",
            "warnings",
            "events_dropped",
        ] {
            assert!(body.get(key).is_some(), "missing overview field {key}");
        }
    }

    #[tokio::test]
    async fn rejects_non_local_host_header() {
        use tower::ServiceExt;
        let app = build_test_app().await;
        let bad = axum::http::Request::builder()
            .uri("/api/health")
            .header("host", "evil.example.com")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(bad).await.expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = json_body(response).await;
        assert_eq!(body["code"], "NON_LOCAL_HOST");
    }

    #[tokio::test]
    async fn unknown_entity_kind_returns_400() {
        use tower::ServiceExt;
        let app = build_test_app().await;
        let response = app
            .oneshot(local_request("/api/entities/bogus/abc"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = json_body(response).await;
        assert_eq!(body["code"], "INVALID_ENTITY_KIND");
    }
}
