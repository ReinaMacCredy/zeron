use super::*;

use http_body_util::StreamBody;
use hyper::body::Frame;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// One raw HTTP/1.1 exchange against `path`: (status, head, body as sent — chunked or not).
async fn exchange_path(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: &str,
) -> (u16, String, String) {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        request.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut raw))
        .await
        .expect("answered in time")
        .unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, head.to_ascii_lowercase(), body.to_owned())
}

fn bearer(token: &str) -> (&'static str, String) {
    ("Authorization", format!("Bearer {token}"))
}

fn accept() -> (&'static str, String) {
    ("Accept", "application/json, text/event-stream".into())
}

#[derive(Default)]
struct RecordingProxy {
    seen: Mutex<Vec<(String, Method, HeaderMap, Bytes)>>,
}

impl ExternalMcpProxy for RecordingProxy {
    fn forward(
        &self,
        req: ExternalProxyRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ExternalProxyResponse, String>> + Send>,
    > {
        let server_id = req.server_id.clone();
        let method = req.method.clone();
        lock(&self.seen).push((req.server_id, req.method, req.headers, req.body));
        Box::pin(async move {
            if server_id == "boom" {
                return Err("upstream unreachable".into());
            }
            if server_id == "sse-srv" {
                let mut headers = HeaderMap::new();
                headers.insert(
                    "content-type",
                    HeaderValue::from_static("text/event-stream"),
                );
                headers.insert("cache-control", HeaderValue::from_static("no-cache"));
                headers.insert("mcp-session-id", HeaderValue::from_static("sess-sse-1"));
                headers.insert(
                    "mcp-protocol-version",
                    HeaderValue::from_static("2025-03-26"),
                );
                let (tx, rx) = mpsc::channel::<Bytes>(4);
                tokio::spawn(async move {
                    let _ = tx
                        .send(Bytes::from_static(b"event: message\ndata: {\"ok\":1}\n\n"))
                        .await;
                    let _ = tx
                        .send(Bytes::from_static(b"event: message\ndata: {\"ok\":2}\n\n"))
                        .await;
                });
                let stream = futures::stream::unfold(rx, |mut rx| async move {
                    rx.recv()
                        .await
                        .map(|chunk| (Ok::<_, Infallible>(Frame::data(chunk)), rx))
                });
                return Ok(ExternalProxyResponse {
                    status: StatusCode::OK,
                    headers,
                    body: BodyExt::boxed_unsync(StreamBody::new(stream)),
                });
            }
            let mut headers = HeaderMap::new();
            headers.insert("content-type", HeaderValue::from_static("application/json"));
            headers.insert("mcp-session-id", HeaderValue::from_static("sess-json-42"));
            headers.insert(
                "mcp-protocol-version",
                HeaderValue::from_static("2025-11-25"),
            );
            let payload = json!({
                "server": server_id,
                "method": method.as_str(),
            })
            .to_string();
            Ok(ExternalProxyResponse {
                status: StatusCode::OK,
                headers,
                body: Full::new(Bytes::from(payload)).boxed_unsync(),
            })
        })
    }
}

#[tokio::test]
async fn external_proxy_routes_enforces_auth_and_passes_headers_and_sse() {
    let proxy = Arc::new(RecordingProxy::default());
    let server = McpHttpServer::bind(proxy.clone()).await.unwrap();
    let grant = server.grant().unwrap();
    let port = server.port();

    assert_eq!(
        server.server_url("linear_1.0"),
        format!("http://127.0.0.1:{port}/mcp/server/linear_1.0")
    );

    let (status, head, _) = exchange_path(port, "POST", "/mcp/server/linear", &[], "{}").await;
    assert_eq!(status, 401, "missing bearer token");
    assert!(head.contains("www-authenticate: bearer"));

    let (status, ..) = exchange_path(
        port,
        "POST",
        "/mcp/server/linear",
        &[bearer("wrong-token")],
        "{}",
    )
    .await;
    assert_eq!(status, 401, "wrong bearer token");

    let (status, ..) = exchange_path(
        port,
        "POST",
        "/mcp/server/linear",
        &[
            bearer(grant.token()),
            ("Origin", "https://evil.example".into()),
        ],
        "{}",
    )
    .await;
    assert_eq!(status, 403, "origin rejected");

    let (status, ..) = exchange_path(
        port,
        "POST",
        "/mcp/server/linear",
        &[
            bearer(grant.token()),
            ("Host", format!("evil.example:{port}")),
        ],
        "{}",
    )
    .await;
    assert_eq!(status, 403, "rebound host rejected");

    for bad in [
        "/mcp",
        "/mcp/server/",
        "/mcp/server/..",
        "/mcp/server/a/b",
        "/mcp/server/bad$id",
    ] {
        let (status, ..) = exchange_path(port, "POST", bad, &[bearer(grant.token())], "{}").await;
        assert_eq!(status, 404, "{bad}");
    }

    let (status, head, _) = exchange_path(
        port,
        "PUT",
        "/mcp/server/linear",
        &[bearer(grant.token())],
        "{}",
    )
    .await;
    assert_eq!(status, 405);
    assert!(head.contains("allow: get, post, delete"));

    let (status, head, body) = exchange_path(
        port,
        "POST",
        "/mcp/server/linear-v2.0",
        &[
            bearer(grant.token()),
            ("Mcp-Session-Id", "client-sess".into()),
            accept(),
        ],
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert!(head.contains("content-type: application/json"), "{head}");
    assert!(head.contains("mcp-session-id: sess-json-42"), "{head}");
    assert!(head.contains("mcp-protocol-version: 2025-11-25"), "{head}");
    assert!(body.contains("\"server\":\"linear-v2.0\""), "{body}");

    let (status, ..) = exchange_path(
        port,
        "GET",
        "/mcp/server/linear-v2.0",
        &[bearer(grant.token())],
        "",
    )
    .await;
    assert_eq!(status, 200);

    let (status, ..) = exchange_path(
        port,
        "DELETE",
        "/mcp/server/linear-v2.0",
        &[bearer(grant.token())],
        "",
    )
    .await;
    assert_eq!(status, 200);

    let (status, head, body) = exchange_path(
        port,
        "POST",
        "/mcp/server/sse-srv",
        &[bearer(grant.token()), accept()],
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call"}"#,
    )
    .await;
    assert_eq!(status, 200);
    assert!(head.contains("content-type: text/event-stream"), "{head}");
    assert!(head.contains("cache-control: no-cache"), "{head}");
    assert!(head.contains("mcp-session-id: sess-sse-1"), "{head}");
    assert!(head.contains("mcp-protocol-version: 2025-03-26"), "{head}");
    assert!(body.contains("data: {\"ok\":1}"), "{body}");
    assert!(body.contains("data: {\"ok\":2}"), "{body}");

    let (status, _, body) = exchange_path(
        port,
        "POST",
        "/mcp/server/boom",
        &[bearer(grant.token())],
        "{}",
    )
    .await;
    assert_eq!(status, 502);
    assert!(body.contains("upstream unreachable"), "{body}");

    let seen = lock(&proxy.seen);
    assert_eq!(seen.len(), 5);
    assert_eq!(seen[0].0, "linear-v2.0");
    assert_eq!(seen[0].1, Method::POST);
    assert!(
        seen[0].2.get("authorization").is_none(),
        "loopback grant token must not leak"
    );
    assert_eq!(
        seen[0]
            .2
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok()),
        Some("client-sess")
    );
    assert_eq!(seen[1].1, Method::GET);
    assert!(seen[1].3.is_empty());
    assert_eq!(seen[2].1, Method::DELETE);
    assert!(seen[2].3.is_empty());
}

#[tokio::test]
async fn a_dropped_grant_stops_its_token() {
    let server = McpHttpServer::bind(Arc::new(RecordingProxy::default()))
        .await
        .unwrap();
    let grant = server.grant().unwrap();
    let token = grant.token().to_owned();
    assert_eq!(server.live_grants(), 1);
    let (status, ..) = exchange_path(
        server.port(),
        "POST",
        "/mcp/server/linear",
        &[bearer(&token)],
        "{}",
    )
    .await;
    assert_eq!(status, 200);
    drop(grant);
    assert_eq!(server.live_grants(), 0);
    let (status, ..) = exchange_path(
        server.port(),
        "POST",
        "/mcp/server/linear",
        &[bearer(&token)],
        "{}",
    )
    .await;
    assert_eq!(status, 401);
}
