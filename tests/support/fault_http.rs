//! Raw-TCP scripted-fault HTTP server for transport tests.
//!
//! Each harness binds its own `127.0.0.1` port. Every accepted connection consumes the next
//! scripted [`Reply`] and carries at most one request; responses close the connection. A
//! connection with no reply left is closed without a response. Every request is recorded with
//! the (possibly paused) tokio clock reading at which it arrived.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::Instant;

/// What the harness does with one connection.
pub enum Reply {
    /// A complete response with `Content-Length`. `body` is sent byte for byte.
    Respond {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: Vec<u8>,
    },
    /// A complete response sent with `Transfer-Encoding: chunked` in 4 KiB chunks, so the
    /// client cannot learn its size in advance. An addition to the §10.4 table, for the
    /// streamed over-limit case.
    RespondChunked { status: u16, body: Vec<u8> },
    /// Response loss: read the request, then close without responding.
    DropAfterRequest,
    /// Send failure: close as soon as the connection is accepted, before reading anything.
    CloseOnAccept,
    /// Close mid-response: announce `declared` bytes in `Content-Length`, send `sent` of them,
    /// then close. `status` is an addition to the §10.4 table so the truncation happens after
    /// a known status line.
    TruncateBody {
        status: u16,
        declared: usize,
        sent: usize,
    },
    /// Stalled peer: read the request, then hold the connection open silently until the
    /// harness is dropped.
    Stall,
}

impl Reply {
    /// A JSON response with `body` unchanged.
    pub fn json(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::Respond {
            status,
            headers: vec![("content-type", "application/json".to_owned())],
            body: body.into(),
        }
    }

    /// A plain-text response, as a gateway error page would be.
    pub fn text(status: u16, body: &str) -> Self {
        Self::Respond {
            status,
            headers: vec![("content-type", "text/plain".to_owned())],
            body: body.as_bytes().to_vec(),
        }
    }
}

/// One request as received by the harness.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    /// Request target: path plus any query string.
    pub target: String,
    /// Header names are lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Tokio clock reading when the request head and body had been read.
    pub at: Instant,
}

impl RecordedRequest {
    /// First value of header `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A running fault server; dropping it aborts every connection, stalled ones included.
pub struct FaultHttp {
    addr: std::net::SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    connections: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl FaultHttp {
    /// Starts serving `replies`, one per accepted connection, in order.
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let (recorded, accepted) = (requests.clone(), connections.clone());
        let task = tokio::spawn(async move {
            // Owning the per-connection tasks here aborts them when this task is aborted.
            let mut tasks = JoinSet::new();
            let mut replies = replies.into_iter();
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                accepted.fetch_add(1, Ordering::SeqCst);
                tasks.spawn(serve(stream, replies.next(), recorded.clone()));
            }
        });
        Self {
            addr,
            requests,
            connections,
            task,
        }
    }

    /// `http://127.0.0.1:<port>`, with no trailing slash.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Requests received so far, in arrival order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Connections accepted so far, including ones closed before a request was read.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

impl Drop for FaultHttp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A loopback base URL on which nothing is listening: connecting is refused. The port is
/// released before use, so another test could in principle bind it in between; the window is
/// accepted.
pub async fn refused_base_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

async fn serve(
    mut stream: TcpStream,
    reply: Option<Reply>,
    recorded: Arc<Mutex<Vec<RecordedRequest>>>,
) {
    if matches!(reply, Some(Reply::CloseOnAccept)) {
        return;
    }
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    recorded.lock().unwrap().push(request);
    match reply {
        None | Some(Reply::CloseOnAccept) | Some(Reply::DropAfterRequest) => {}
        Some(Reply::Respond {
            status,
            headers,
            body,
        }) => {
            let mut head = status_line(status);
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str(&format!(
                "content-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            ));
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        }
        Some(Reply::RespondChunked { status, body }) => {
            let head = format!(
                "{}content-type: application/json\r\ntransfer-encoding: chunked\r\n\
                 connection: close\r\n\r\n",
                status_line(status)
            );
            let _ = stream.write_all(head.as_bytes()).await;
            for chunk in body.chunks(4096) {
                let size = format!("{:x}\r\n", chunk.len());
                if stream.write_all(size.as_bytes()).await.is_err()
                    || stream.write_all(chunk).await.is_err()
                    || stream.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
            }
            let _ = stream.write_all(b"0\r\n\r\n").await;
        }
        Some(Reply::TruncateBody {
            status,
            declared,
            sent,
        }) => {
            let head = format!(
                "{}content-type: application/json\r\ncontent-length: {declared}\r\n\
                 connection: close\r\n\r\n",
                status_line(status)
            );
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&vec![b' '; sent.min(declared)]).await;
        }
        Some(Reply::Stall) => std::future::pending::<()>().await,
    }
    let _ = stream.shutdown().await;
}

fn status_line(status: u16) -> String {
    format!("HTTP/1.1 {status} Harness\r\n")
}

/// Reads one request head and a `Content-Length` body. Chunked request bodies are not
/// supported; `None` means the peer closed before a full head arrived.
async fn read_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    let mut buf = Vec::new();
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_owned();
    let target = request_line.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let len = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some(RecordedRequest {
        method,
        target,
        headers,
        body,
        at: Instant::now(),
    })
}
