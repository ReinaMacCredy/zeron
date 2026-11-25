//! The engine's loopback gateway to managed external MCP servers
//! (`/mcp/server/{id}`); Zeron's own tools stay on the per-run stdio `zeron mcp`.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::header::{HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{Value, json};

const INVALID_REQUEST: i64 = -32600;
const MAX_BODY: usize = 8 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 128;
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

type Body = UnsyncBoxBody<Bytes, Infallible>;

/// An authenticated loopback request targeting an external managed MCP server
/// (`/mcp/server/{server_id}`).
pub struct ExternalProxyRequest {
    pub server_id: String,
    pub method: hyper::Method,
    pub headers: hyper::HeaderMap,
    pub body: bytes::Bytes,
}

/// Response returned by an [`ExternalMcpProxy`] (JSON or streamed SSE).
pub struct ExternalProxyResponse {
    pub status: hyper::StatusCode,
    pub headers: hyper::HeaderMap,
    pub body: UnsyncBoxBody<bytes::Bytes, std::convert::Infallible>,
}

/// Forwards authenticated `/mcp/server/{server_id}` requests to upstream MCP
/// servers with managed credentials injected by the engine.
pub trait ExternalMcpProxy: Send + Sync {
    fn forward(
        &self,
        req: ExternalProxyRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ExternalProxyResponse, String>> + Send>,
    >;
}

struct GrantState {
    token: String,
}

struct Inner {
    port: u16,
    external: Arc<dyn ExternalMcpProxy>,
    grants: Mutex<Vec<Arc<GrantState>>>,
}

/// A run's access to the gateway: its token works until this drops.
pub struct Grant {
    inner: Weak<Inner>,
    state: Arc<GrantState>,
}

impl Grant {
    /// The bearer token (`Authorization: Bearer …`).
    pub fn token(&self) -> &str {
        &self.state.token
    }
}

impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant").finish_non_exhaustive()
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            lock(&inner.grants).retain(|grant| !Arc::ptr_eq(grant, &self.state));
        }
    }
}

/// The engine's MCP gateway endpoint. Dropping it stops the listener.
pub struct McpHttpServer {
    inner: Arc<Inner>,
    accept: tokio::task::JoinHandle<()>,
}

impl Drop for McpHttpServer {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

impl McpHttpServer {
    /// Bind `127.0.0.1` on a free port and forward `/mcp/server/{id}`
    /// requests through `external` until dropped.
    pub async fn bind(external: Arc<dyn ExternalMcpProxy>) -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let inner = Arc::new(Inner {
            port,
            external,
            grants: Mutex::new(Vec::new()),
        });
        let weak = Arc::downgrade(&inner);
        let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
        let accept = tokio::spawn(async move {
            loop {
                let Ok(slot) = slots.clone().acquire_owned().await else {
                    return;
                };
                let (stream, peer) = match listener.accept().await {
                    Ok(accepted) => accepted,
                    Err(err) => {
                        tracing::warn!(error = %err, "mcp gateway: accept failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let Some(inner) = weak.upgrade() else { return };
                tokio::spawn(async move {
                    serve_socket(inner, stream, peer).await;
                    drop(slot);
                });
            }
        });
        tracing::info!(port, "MCP gateway listening");
        Ok(Self { inner, accept })
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    /// `http://127.0.0.1:{port}/mcp/server/{server_id}`.
    pub fn server_url(&self, server_id: &str) -> String {
        format!(
            "http://127.0.0.1:{}/mcp/server/{}",
            self.inner.port, server_id
        )
    }

    /// A fresh token for one run.
    pub fn grant(&self) -> std::io::Result<Grant> {
        let token = zeron_rpc::ipc_auth::generate_token()?;
        let state = Arc::new(GrantState { token });
        lock(&self.inner.grants).push(state.clone());
        Ok(Grant {
            inner: Arc::downgrade(&self.inner),
            state,
        })
    }

    /// How many runs hold a live token (tests, diagnostics).
    pub fn live_grants(&self) -> usize {
        lock(&self.inner.grants).len()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn serve_socket(inner: Arc<Inner>, stream: tokio::net::TcpStream, peer: SocketAddr) {
    if !peer.ip().is_loopback() {
        return;
    }
    let _ = stream.set_nodelay(true);
    let service = hyper::service::service_fn(move |request| {
        let inner = inner.clone();
        async move { Ok::<_, Infallible>(handle(inner, request).await) }
    });
    let io = hyper_util::rt::TokioIo::new(stream);
    // (The timer is what makes the header timeout apply.)
    if let Err(err) = hyper::server::conn::http1::Builder::new()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT)
        .keep_alive(true)
        .serve_connection(io, service)
        .await
    {
        tracing::debug!(error = %err, "mcp gateway: connection ended");
    }
}

fn full(bytes: impl Into<Bytes>) -> Body {
    Full::new(bytes.into()).boxed_unsync()
}

fn empty(status: StatusCode) -> Response<Body> {
    let mut response = Response::new(full(Bytes::new()));
    *response.status_mut() = status;
    response
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn json_response(status: StatusCode, value: &Value) -> Response<Body> {
    let mut response = Response::new(full(value.to_string()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert("content-type", HeaderValue::from_static("application/json"));
    response
}

fn refusal(status: StatusCode, message: &str) -> Response<Body> {
    json_response(
        status,
        &error_response(Value::Null, INVALID_REQUEST, message),
    )
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// The `Host` a loopback client sends: `127.0.0.1`, `localhost` or `[::1]`,
/// on this port. Anything else came through a name that resolved here
/// (DNS rebinding).
fn host_allowed(headers: &HeaderMap, port: u16) -> bool {
    let Some(host) = header(headers, "host") else {
        return false;
    };
    let host = host.trim().to_ascii_lowercase();
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|name| host == format!("{name}:{port}"))
}

/// Constant-time equality, so a wrong token's timing says nothing about a
/// right one.
fn same_token(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn has_grant(inner: &Inner, headers: &HeaderMap) -> bool {
    let Some(presented) = header(headers, "authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
    else {
        return false;
    };
    // Every grant compared, so the match's position leaks nothing either.
    lock(&inner.grants).iter().fold(false, |found, grant| {
        same_token(&grant.token, presented) | found
    })
}

fn external_server_id(path: &str) -> Option<&str> {
    let id = path.strip_prefix("/mcp/server/")?;
    if id.is_empty()
        || id == "."
        || id == ".."
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return None;
    }
    Some(id)
}

fn is_hop_by_hop_response_header(name: &hyper::header::HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
    )
}

async fn handle_external(
    inner: &Inner,
    server_id: String,
    mut headers: HeaderMap,
    request: Request<hyper::body::Incoming>,
) -> Response<Body> {
    let method = request.method().clone();
    if !matches!(method, Method::POST | Method::GET | Method::DELETE) {
        let mut response = empty(StatusCode::METHOD_NOT_ALLOWED);
        response
            .headers_mut()
            .insert("allow", HeaderValue::from_static("GET, POST, DELETE"));
        return response;
    }
    let body = if method == Method::POST {
        match http_body_util::Limited::new(request.into_body(), MAX_BODY)
            .collect()
            .await
        {
            Ok(collected) => collected.to_bytes(),
            Err(_) => return refusal(StatusCode::PAYLOAD_TOO_LARGE, "request too large"),
        }
    } else {
        Bytes::new()
    };
    // Never forward the loopback run grant token to an upstream MCP server.
    headers.remove(hyper::header::AUTHORIZATION);
    let server_id_for_log = server_id.clone();
    match inner
        .external
        .forward(ExternalProxyRequest {
            server_id,
            method,
            headers,
            body,
        })
        .await
    {
        Ok(ext) => {
            let mut response = Response::new(ext.body);
            *response.status_mut() = ext.status;
            let mut last_name: Option<hyper::header::HeaderName> = None;
            for (name, value) in ext.headers {
                let name = match name {
                    Some(n) => {
                        last_name = Some(n.clone());
                        n
                    }
                    None => match last_name.clone() {
                        Some(n) => n,
                        None => continue,
                    },
                };
                if is_hop_by_hop_response_header(&name) {
                    continue;
                }
                response.headers_mut().append(name, value);
            }
            response
        }
        Err(err) => {
            // (The agent's MCP client drops a server that fails quietly: said
            // here, or nothing anywhere says why its tools are missing.)
            tracing::warn!(server = %server_id_for_log, error = %err, "mcp gateway: forward failed");
            json_response(
                StatusCode::BAD_GATEWAY,
                &error_response(Value::Null, -32603, &err),
            )
        }
    }
}

async fn handle(inner: Arc<Inner>, request: Request<hyper::body::Incoming>) -> Response<Body> {
    let headers = request.headers().clone();
    // Any browser page carries `Origin`; nothing that should reach this does.
    if headers.contains_key("origin") {
        return refusal(StatusCode::FORBIDDEN, "origin not allowed");
    }
    if !host_allowed(&headers, inner.port) {
        return refusal(StatusCode::FORBIDDEN, "host not allowed");
    }
    let Some(server_id) = external_server_id(request.uri().path()).map(str::to_owned) else {
        return empty(StatusCode::NOT_FOUND);
    };
    if !has_grant(&inner, &headers) {
        tracing::warn!(server = %server_id, "mcp gateway: refused a request without a run's token");
        let mut response = refusal(StatusCode::UNAUTHORIZED, "missing or wrong token");
        response
            .headers_mut()
            .insert("www-authenticate", HeaderValue::from_static("Bearer"));
        return response;
    }
    handle_external(&inner, server_id, headers, request).await
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
