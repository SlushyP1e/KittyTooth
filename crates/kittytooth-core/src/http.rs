//! Minimal HTTP/1.1 helpers for the local overlay and OSCQuery servers. Only what those
//! need: GET request line parsing, simple responses and a tiny GET client.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{KtError, KtResult};

const MAX_HEADER: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
}

/// Reads one request head (we never accept bodies).
pub async fn read_request<S: AsyncRead + Unpin>(s: &mut S) -> std::io::Result<Request> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let read = async {
        loop {
            let n = s.read(&mut chunk).await?;
            if n == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof"));
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.windows(2).any(|w| w == b"\n\n") {
                return Ok(());
            }
            if buf.len() > MAX_HEADER {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "header too large"));
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout"))??;
    parse_request_line(&buf).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad request line"))
}

pub fn parse_request_line(buf: &[u8]) -> Option<Request> {
    let text = std::str::from_utf8(buf).ok()?;
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    parts.next().filter(|v| v.starts_with("HTTP/"))?;
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target.to_string(), None),
    };
    Some(Request { method, path, query })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    }
}

/// A complete response with `Connection: close`.
pub fn response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

/// Head of a Server-Sent Events stream.
pub fn sse_head() -> &'static [u8] {
    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: keep-alive\r\n\r\n"
}

/// Simple HTTP GET returning (status, body). Supports Content-Length, chunked and
/// read-to-close bodies.
pub async fn get(addr: SocketAddr, path_and_query: &str, timeout: Duration) -> KtResult<(u16, Vec<u8>)> {
    let fut = async {
        let mut s = TcpStream::connect(addr).await?;
        let req = format!("GET {path_and_query} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAccept: application/json\r\n\r\n");
        s.write_all(req.as_bytes()).await?;
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await?;
        Ok::<_, std::io::Error>(buf)
    };
    let buf = tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| KtError::Io(format!("HTTP GET {addr} timed out")))??;
    parse_response(&buf).ok_or_else(|| KtError::Io(format!("bad HTTP response from {addr}")))
}

pub fn parse_response(buf: &[u8]) -> Option<(u16, Vec<u8>)> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&buf[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let mut chunked = false;
    let mut length: Option<usize> = None;
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            if k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            } else if k == "content-length" {
                length = v.parse().ok();
            }
        }
    }
    let body = &buf[end + 4..];
    let body = if chunked {
        dechunk(body)?
    } else if let Some(n) = length {
        body.get(..n)?.to_vec()
    } else {
        body.to_vec()
    };
    Some((status, body))
}

fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let le = b.windows(2).position(|w| w == b"\r\n")?;
        let size_str = std::str::from_utf8(&b[..le]).ok()?;
        let size = usize::from_str_radix(size_str.split(';').next()?.trim(), 16).ok()?;
        b = &b[le + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..size)?);
        b = b.get(size + 2..)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_line() {
        let r = parse_request_line(b"GET /overlay?size=2 HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/overlay");
        assert_eq!(r.query.as_deref(), Some("size=2"));
        assert!(parse_request_line(b"garbage").is_none());
    }

    #[test]
    fn responses() {
        let r = response(200, "text/plain", b"hi");
        let (s, b) = parse_response(&r).unwrap();
        assert_eq!(s, 200);
        assert_eq!(b, b"hi");
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked).unwrap().1, b"Wikipedia");
        let no_len = b"HTTP/1.0 404 Not Found\r\n\r\nnope";
        assert_eq!(parse_response(no_len).unwrap(), (404, b"nope".to_vec()));
    }
}
