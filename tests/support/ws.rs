//! Scripted loopback WebSocket server for feed tests.
//!
//! Each harness binds its own `127.0.0.1` port; feeds are pointed at [`WsHarness::url`] with the
//! builder's URL override. Every accepted TCP connection consumes the next scripted
//! [`WsConnection`]; a connection with no script left is closed before the handshake. The
//! harness records every handshake request target, every client frame and the pongs that answer
//! its pings.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Message, http};

/// How the harness answers the opening handshake.
pub enum Handshake {
    Accept,
    /// Answer the upgrade request with this HTTP status.
    Reject(u16),
}

/// One scripted action after a successful handshake.
pub enum Step {
    SendBinary(Vec<u8>),
    SendText(String),
    Ping,
    /// Send a close frame (with this code), wait up to a second for the client's close reply
    /// to be recorded, then stop.
    Close(Option<u16>),
    /// Drop the TCP connection without a close frame.
    Eof,
    Wait(Duration),
    /// Wait for the client's next text frame on this connection and record whether it equals
    /// this JSON value.
    ExpectText(serde_json::Value),
    /// Stop reading client frames, keeping the connection open, so the client's writes stall
    /// once the socket buffers fill (see [`WsHarness::start_with_recv_buffer`]).
    StopReading,
}

/// The script for one connection. If `steps` does not end in `Close` or `Eof`, the harness
/// becomes a silent peer that keeps the connection open, still recording client frames.
pub struct WsConnection {
    pub handshake: Handshake,
    pub steps: Vec<Step>,
}

impl WsConnection {
    /// Accept, then run `steps`.
    pub fn accept(steps: Vec<Step>) -> Self {
        WsConnection {
            handshake: Handshake::Accept,
            steps,
        }
    }

    /// Refuse the handshake with `status`.
    pub fn reject(status: u16) -> Self {
        WsConnection {
            handshake: Handshake::Reject(status),
            steps: Vec::new(),
        }
    }
}

/// One frame a client sent.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientFrame {
    Text(String),
    Binary(Vec<u8>),
    Ping,
    Pong,
    Close(Option<u16>),
}

#[derive(Default)]
struct Recorded {
    /// Request targets (path plus query), in arrival order.
    targets: Vec<String>,
    /// Client frames per connection, by connection index.
    frames: Vec<Vec<ClientFrame>>,
    /// Failed `ExpectText` steps.
    mismatches: Vec<String>,
    connections: usize,
}

pub struct WsHarness {
    addr: std::net::SocketAddr,
    recorded: Arc<Mutex<Recorded>>,
    task: JoinHandle<()>,
}

impl WsHarness {
    /// Starts serving `connections`, one per accepted TCP connection, in order.
    pub async fn start(connections: Vec<WsConnection>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        Self::serve_on(listener, connections)
    }

    /// Like [`WsHarness::start`], with a receive buffer of about `bytes` on every accepted
    /// socket, so a [`Step::StopReading`] stalls the client after little data.
    pub async fn start_with_recv_buffer(connections: Vec<WsConnection>, bytes: u32) -> Self {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        // Set before listen: accepted sockets inherit it.
        socket.set_recv_buffer_size(bytes).unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        Self::serve_on(socket.listen(16).unwrap(), connections)
    }

    fn serve_on(listener: TcpListener, connections: Vec<WsConnection>) -> Self {
        let addr = listener.local_addr().unwrap();
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let shared = Arc::clone(&recorded);
        let task = tokio::spawn(async move {
            let mut served = JoinSet::new();
            let mut connections = connections.into_iter();
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let index = {
                    let mut r = shared.lock().unwrap();
                    r.connections += 1;
                    r.frames.push(Vec::new());
                    r.connections - 1
                };
                if let Some(connection) = connections.next() {
                    served.spawn(serve(stream, index, connection, Arc::clone(&shared)));
                }
            }
        });
        WsHarness {
            addr,
            recorded,
            task,
        }
    }

    /// `ws://127.0.0.1:<port>`.
    pub fn url(&self) -> url::Url {
        url::Url::parse(&format!("ws://{}", self.addr)).unwrap()
    }

    /// Handshake request targets received so far.
    pub fn targets(&self) -> Vec<String> {
        self.recorded.lock().unwrap().targets.clone()
    }

    /// TCP connections accepted so far.
    pub fn connections(&self) -> usize {
        self.recorded.lock().unwrap().connections
    }

    /// Frames the client sent on connection `index` (0-based).
    pub fn frames(&self, index: usize) -> Vec<ClientFrame> {
        self.recorded
            .lock()
            .unwrap()
            .frames
            .get(index)
            .cloned()
            .unwrap_or_default()
    }

    /// Text frames the client sent on connection `index`, parsed as JSON.
    pub fn texts(&self, index: usize) -> Vec<serde_json::Value> {
        self.frames(index)
            .into_iter()
            .filter_map(|f| match f {
                ClientFrame::Text(t) => serde_json::from_str(&t).ok(),
                _ => None,
            })
            .collect()
    }

    /// `ExpectText` steps that saw a different frame.
    pub fn mismatches(&self) -> Vec<String> {
        self.recorded.lock().unwrap().mismatches.clone()
    }
}

impl Drop for WsHarness {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Aborts the wrapped task when dropped.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn client_frame(message: Message) -> ClientFrame {
    match message {
        Message::Text(t) => ClientFrame::Text(t.as_str().to_owned()),
        Message::Binary(b) => ClientFrame::Binary(b.to_vec()),
        Message::Ping(_) => ClientFrame::Ping,
        Message::Pong(_) => ClientFrame::Pong,
        Message::Close(c) => ClientFrame::Close(c.map(|c| u16::from(c.code))),
        Message::Frame(_) => ClientFrame::Binary(Vec::new()),
    }
}

async fn serve(
    stream: TcpStream,
    index: usize,
    connection: WsConnection,
    recorded: Arc<Mutex<Recorded>>,
) {
    let reject = match connection.handshake {
        Handshake::Accept => None,
        Handshake::Reject(status) => Some(status),
    };
    // The callback's result shape is fixed by tungstenite, so the large error cannot be boxed.
    #[allow(clippy::result_large_err)]
    let on_handshake = {
        let recorded = Arc::clone(&recorded);
        move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            recorded
                .lock()
                .unwrap()
                .targets
                .push(request.uri().to_string());
            match reject {
                None => Ok(response),
                Some(status) => Err(http::Response::builder().status(status).body(None).unwrap()),
            }
        }
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, on_handshake).await else {
        return;
    };
    let (mut sink, mut source) = ws.split();
    let arrived = Arc::new(Notify::new());
    let mut reader = Some(AbortOnDrop(tokio::spawn({
        let recorded = Arc::clone(&recorded);
        let arrived = Arc::clone(&arrived);
        async move {
            while let Some(Ok(message)) = source.next().await {
                recorded.lock().unwrap().frames[index].push(client_frame(message));
                arrived.notify_one();
            }
        }
    })));
    let mut texts_seen = 0;
    for step in connection.steps {
        match step {
            Step::SendBinary(bytes) => {
                if sink.send(Message::Binary(bytes.into())).await.is_err() {
                    return;
                }
            }
            Step::SendText(text) => {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    return;
                }
            }
            Step::Ping => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
            Step::Close(code) => {
                let frame = code.map(|c| CloseFrame {
                    code: CloseCode::from(c),
                    reason: "".into(),
                });
                let _ = sink.send(Message::Close(frame)).await;
                let replied = async {
                    loop {
                        let woken = arrived.notified();
                        let closed = recorded.lock().unwrap().frames[index]
                            .iter()
                            .any(|f| matches!(f, ClientFrame::Close(_)));
                        if closed {
                            return;
                        }
                        woken.await;
                    }
                };
                let _ = tokio::time::timeout(Duration::from_secs(1), replied).await;
                return;
            }
            Step::StopReading => reader = None,
            Step::Eof => {
                drop(reader);
                drop(sink);
                return;
            }
            Step::Wait(d) => tokio::time::sleep(d).await,
            Step::ExpectText(expected) => loop {
                let woken = arrived.notified();
                let next = {
                    let r = recorded.lock().unwrap();
                    r.frames[index]
                        .iter()
                        .filter_map(|f| match f {
                            ClientFrame::Text(t) => Some(t.clone()),
                            _ => None,
                        })
                        .nth(texts_seen)
                };
                if let Some(text) = next {
                    texts_seen += 1;
                    let got: Option<serde_json::Value> = serde_json::from_str(&text).ok();
                    if got.as_ref() != Some(&expected) {
                        recorded.lock().unwrap().mismatches.push(format!(
                            "connection {index}: expected {expected}, got {text}"
                        ));
                    }
                    break;
                }
                woken.await;
            },
        }
    }
    let _keep_open = (sink, reader);
    std::future::pending::<()>().await;
}
