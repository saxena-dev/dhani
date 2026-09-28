//! Local servers for the examples. Nothing here reaches the network or reads an environment
//! variable: the REST examples talk to a `wiremock` server and the feed examples to a loopback
//! WebSocket server, both on `127.0.0.1` with a free port. The logic is copied from the test
//! support, not linked, so each example stays self-contained.
#![allow(dead_code)]

#[cfg(feature = "rest")]
use dhani::config::{Environment, Urls};
use dhani::{AccessToken, ClientId, Credentials};

/// Placeholder credentials: the local servers accept anything.
pub fn credentials() -> Credentials {
    Credentials::new(
        ClientId::new("1000000009").unwrap(),
        AccessToken::new("example-access-token").unwrap(),
    )
}

/// Every REST host pointed at `server` (the API under `/v2`, the auth host at its root).
#[cfg(feature = "rest")]
pub fn urls_for(server: &wiremock::MockServer) -> Urls {
    let base = server.uri();
    let url = |path: &str| url::Url::parse(&format!("{base}{path}")).expect("mock URL");
    let mut urls = Urls::for_env(Environment::Live);
    urls.rest = url("/v2");
    urls.auth = url("");
    urls
}

/// An HTTP client that ignores `HTTP_PROXY` and friends, so a proxy setting cannot send the
/// local mock traffic elsewhere. Pass it with `DhanClient::builder().http_client(..)`.
#[cfg(feature = "rest")]
pub fn local_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("a local HTTP client")
}

/// The loopback WebSocket side, used by the feed examples.
#[cfg(feature = "feed")]
pub mod ws {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    /// A market-feed Ticker packet (code 2) for NSE_EQ `security_id`, built from the decoder's
    /// public layout constants.
    pub fn ticker(security_id: u32, ltp: f32, ltt: u32) -> Vec<u8> {
        use dhani::decoder::layout::*;
        let mut p = vec![0u8; TICKER_LEN];
        p[H_CODE] = CODE_TICKER;
        p[H_LEN..H_LEN + 2].copy_from_slice(&u16::try_from(TICKER_LEN).unwrap().to_le_bytes());
        p[H_SEGMENT] = 1; // NSE_EQ
        p[H_SECURITY_ID..H_SECURITY_ID + 4].copy_from_slice(&security_id.to_le_bytes());
        p[TICKER_LTP..TICKER_LTP + 4].copy_from_slice(&ltp.to_le_bytes());
        p[TICKER_LTT..TICKER_LTT + 4].copy_from_slice(&ltt.to_le_bytes());
        p
    }

    /// What a loopback server does after the handshake.
    pub enum Send {
        /// Wait for the client's next text message (a subscription or a login).
        AwaitText,
        Binary(Vec<u8>),
        Text(String),
        Pause(Duration),
    }

    /// A loopback WebSocket server for exactly one connection (a reconnect would get no answer)
    /// that runs `script`, then keeps the connection open. Returns its `ws://` URL and the
    /// client's text messages as they arrive.
    pub async fn ws_server(script: Vec<Send>) -> (url::Url, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = url::Url::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
                return;
            };
            let (mut sink, mut source) = ws.split();
            let mut read_text = async |log: &Arc<Mutex<Vec<String>>>| {
                while let Some(Ok(message)) = source.next().await {
                    if let Message::Text(text) = message {
                        log.lock().unwrap().push(text.as_str().to_owned());
                        return;
                    }
                }
            };
            for step in script {
                match step {
                    Send::AwaitText => read_text(&log).await,
                    Send::Binary(bytes) => {
                        let _ = sink.send(Message::Binary(bytes.into())).await;
                    }
                    Send::Text(text) => {
                        let _ = sink.send(Message::Text(text.into())).await;
                    }
                    Send::Pause(d) => tokio::time::sleep(d).await,
                }
            }
            // Keep the connection open (answering pings) until the client closes it.
            while let Some(Ok(message)) = source.next().await {
                if let Message::Text(text) = message {
                    log.lock().unwrap().push(text.as_str().to_owned());
                }
            }
        });
        (url, received)
    }
}
