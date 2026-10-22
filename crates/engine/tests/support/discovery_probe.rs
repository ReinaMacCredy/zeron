use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};

const PROBE: &[u8] = b"HEAD / ";

// A running Zeron's preview discovery sends `HEAD /` once to every loopback listener whose cwd is inside an open project, test binaries included.
pub async fn accept(listener: &TcpListener) -> std::io::Result<(TcpStream, SocketAddr)> {
    loop {
        let (stream, peer) = listener.accept().await?;
        if !is_probe(&stream).await {
            return Ok((stream, peer));
        }
    }
}

async fn is_probe(stream: &TcpStream) -> bool {
    let mut head = [0u8; PROBE.len()];
    loop {
        match stream.peek(&mut head).await {
            Ok(n) if n == PROBE.len() => return head == PROBE,
            Ok(n) if n > 0 && PROBE.starts_with(&head[..n]) => {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            _ => return false,
        }
    }
}
