use std::cell::Cell;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::watch;
use zeron_proto::computer::{
    CUA_MAX_REQUEST_BYTES, CUA_MAX_RESPONSE_BYTES, CuaEffect, CuaReply, CuaTransportError,
};

pub(super) struct Abort(watch::Sender<bool>);

impl Abort {
    pub fn new() -> Self {
        Self(watch::Sender::new(false))
    }

    pub fn abort(&self) {
        self.0.send_replace(true);
    }

    pub fn is_aborted(&self) -> bool {
        *self.0.borrow()
    }

    pub async fn wait(&self) {
        let mut aborted = self.0.subscribe();
        let _ = aborted.wait_for(|aborted| *aborted).await;
    }
}

const CANCELLED: &str =
    "Computer operation cancelled. Input already dispatched may have taken effect; do not replay.";
const TIMED_OUT: &str = "Computer request timed out; do not replay an uncertain action.";

fn transport_error(message: impl Into<String>, effect: CuaEffect) -> CuaTransportError {
    CuaTransportError {
        message: message.into(),
        effect,
    }
}

pub(super) async fn cua_request(
    socket: &Path,
    request: &Value,
    timeout: Duration,
    mutation: bool,
    signal: Option<&Abort>,
) -> Result<Value, CuaTransportError> {
    if signal.is_some_and(Abort::is_aborted) {
        return Err(transport_error(
            "Cancelled before dispatch.",
            CuaEffect::NotDispatched,
        ));
    }
    let mut encoded = match serde_json::to_vec(request) {
        Ok(encoded) if encoded.len() < CUA_MAX_REQUEST_BYTES => encoded,
        _ => {
            return Err(transport_error(
                "Invalid or oversized computer request.",
                CuaEffect::NotDispatched,
            ));
        }
    };
    encoded.push(b'\n');
    let dispatched = Cell::new(false);
    let exchange = async {
        let mut stream = UnixStream::connect(socket)
            .await
            .map_err(|e| e.to_string())?;
        if signal.is_some_and(Abort::is_aborted) {
            return Err(CANCELLED.to_string());
        }
        dispatched.set(true);
        stream
            .write_all(&encoded)
            .await
            .map_err(|e| e.to_string())?;
        let mut line = Vec::new();
        BufReader::new(stream)
            .take(CUA_MAX_RESPONSE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|e| e.to_string())?;
        if line.len() > CUA_MAX_RESPONSE_BYTES {
            return Err("Computer response exceeded its byte budget.".to_string());
        }
        if line.pop() != Some(b'\n') {
            return Err("Computer connection closed before a result.".to_string());
        }
        serde_json::from_slice::<Value>(&line).map_err(|_| "Invalid computer response.".to_string())
    };
    let aborted = async {
        match signal {
            Some(signal) => signal.wait().await,
            None => std::future::pending().await,
        }
    };
    let outcome = tokio::select! {
        biased;
        result = exchange => result,
        () = aborted => Err(CANCELLED.to_string()),
        () = tokio::time::sleep(timeout) => Err(TIMED_OUT.to_string()),
    };
    outcome.map_err(|message| {
        let effect = if mutation && dispatched.get() {
            CuaEffect::DispatchedUnknown
        } else {
            CuaEffect::NotDispatched
        };
        transport_error(message, effect)
    })
}

pub(super) fn tool_reply(value: Value, mutation: bool) -> Result<CuaReply, CuaTransportError> {
    serde_json::from_value(value).map_err(|_| {
        transport_error(
            "Invalid computer response.",
            if mutation {
                CuaEffect::DispatchedUnknown
            } else {
                CuaEffect::NotDispatched
            },
        )
    })
}

pub(super) async fn cua_tool_request(
    socket: &Path,
    request: &Value,
    timeout: Duration,
) -> Result<CuaReply, CuaTransportError> {
    tool_reply(
        cua_request(socket, request, timeout, false, None).await?,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::net::UnixListener;

    fn socket_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("s.sock")
    }

    async fn serve_once(listener: UnixListener, reply: &'static [u8], hold: bool) {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut line = String::new();
        let (read, mut write) = stream.split();
        BufReader::new(read).read_line(&mut line).await.unwrap();
        write.write_all(reply).await.unwrap();
        if hold {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    #[tokio::test]
    async fn reads_one_newline_terminated_reply() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(&dir);
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(serve_once(listener, b"{\"ok\":true}\n", false));
        let reply = cua_request(
            &path,
            &json!({ "method": "metadata" }),
            Duration::from_secs(1),
            false,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply, json!({ "ok": true }));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn preserves_multibyte_utf8_split_across_reply_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(&dir);
        let listener = UnixListener::bind(&path).unwrap();
        let reply = "{\"ok\":true,\"text\":\"è🧪\"}\n".as_bytes();
        let split = reply.iter().position(|&b| b == 0xF0).unwrap() + 1;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut line = String::new();
            let (read, mut write) = stream.split();
            BufReader::new(read).read_line(&mut line).await.unwrap();
            write.write_all(&reply[..split]).await.unwrap();
            write.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
            write.write_all(&reply[split..]).await.unwrap();
        });
        let reply = cua_request(
            &path,
            &json!({ "method": "probe" }),
            Duration::from_secs(2),
            false,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply, json!({ "ok": true, "text": "è🧪" }));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_timed_out_mutation_is_dispatched_unknown_and_a_missing_socket_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(&dir);
        let missing = cua_request(&path, &json!({}), Duration::from_secs(1), true, None)
            .await
            .unwrap_err();
        assert_eq!(missing.effect, CuaEffect::NotDispatched);
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(serve_once(listener, b"", true));
        let timed_out = cua_request(&path, &json!({}), Duration::from_millis(100), true, None)
            .await
            .unwrap_err();
        assert_eq!(timed_out.effect, CuaEffect::DispatchedUnknown);
        assert_eq!(timed_out.message, TIMED_OUT);
        server.abort();
    }

    #[tokio::test]
    async fn an_abort_ends_the_wait_and_a_pre_aborted_signal_never_connects() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(&dir);
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(serve_once(listener, b"", true));
        let signal = std::rc::Rc::new(Abort::new());
        let local = tokio::task::LocalSet::new();
        let aborting = signal.clone();
        let error = local
            .run_until(async move {
                tokio::task::spawn_local(async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    aborting.abort();
                });
                cua_request(
                    &path,
                    &json!({}),
                    Duration::from_secs(5),
                    true,
                    Some(&signal),
                )
                .await
                .unwrap_err()
            })
            .await;
        assert_eq!(error.effect, CuaEffect::DispatchedUnknown);
        assert_eq!(error.message, CANCELLED);
        server.abort();
        let aborted = Abort::new();
        aborted.abort();
        let early = cua_request(
            &dir.path().join("none.sock"),
            &json!({}),
            Duration::from_secs(1),
            true,
            Some(&aborted),
        )
        .await
        .unwrap_err();
        assert_eq!(early.effect, CuaEffect::NotDispatched);
    }

    #[tokio::test]
    async fn refuses_oversized_requests_and_unterminated_replies() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(&dir);
        let big = json!({ "text": "x".repeat(CUA_MAX_REQUEST_BYTES) });
        let oversized = cua_request(&path, &big, Duration::from_secs(1), true, None)
            .await
            .unwrap_err();
        assert_eq!(oversized.effect, CuaEffect::NotDispatched);
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(serve_once(listener, b"{\"ok\":true}", false));
        let closed = cua_request(&path, &json!({}), Duration::from_secs(1), false, None)
            .await
            .unwrap_err();
        assert_eq!(
            closed.message,
            "Computer connection closed before a result."
        );
        server.await.unwrap();
    }
}
