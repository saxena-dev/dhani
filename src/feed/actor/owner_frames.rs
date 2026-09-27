//! Frame handling of the owner: decoding, decode-failure sampling, server disconnects and the
//! clean close.

use std::time::SystemTime;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite;
use tracing::{Level, Span};

use super::super::super::protocol::{Decoded, FeedProtocol, Message};
use super::super::super::{
    DisconnectReason, FeedEvent, FeedState, FrameKind, Lifecycle, RawFrame, TerminalReason,
};
use super::super::lifecycle::{Cause, Disposition, dispose};
use super::{Ended, Owner, Socket};
use crate::decoder::{DecodeError, DecodeErrorKind};
use crate::error::DataErrorCode;
use crate::obs::events::{self, emit};
use crate::obs::metrics::{self, DecodeFailure, FrameKind as FrameLabel};
use crate::obs::spans;

fn decode_failure(kind: DecodeErrorKind) -> DecodeFailure {
    match kind {
        DecodeErrorKind::Truncated => DecodeFailure::Truncated,
        DecodeErrorKind::BadLength => DecodeFailure::BadLength,
        DecodeErrorKind::UnknownCode => DecodeFailure::UnknownCode,
        DecodeErrorKind::TrailingBytes => DecodeFailure::TrailingBytes,
        DecodeErrorKind::LengthMismatch => DecodeFailure::LengthMismatch,
        DecodeErrorKind::RowCountMismatch => DecodeFailure::RowCountMismatch,
        DecodeErrorKind::AmbiguousDisconnect => DecodeFailure::AmbiguousDisconnect,
        DecodeErrorKind::UnexpectedBinary => DecodeFailure::UnexpectedBinary,
        DecodeErrorKind::UnexpectedText => DecodeFailure::UnexpectedText,
        _ => DecodeFailure::Json,
    }
}

/// Counts decode failures per connection: the first 10 are reported, then every 1000th.
#[derive(Default)]
pub(super) struct Sampler {
    seen: u64,
    reported: u64,
}

impl Sampler {
    /// Whether this failure is reported, and how many were suppressed before it.
    fn sample(&mut self) -> Option<u64> {
        self.seen += 1;
        if self.seen <= 10 || self.seen.is_multiple_of(1000) {
            let suppressed = self.seen - 1 - self.reported;
            self.reported = self.seen;
            Some(suppressed)
        } else {
            None
        }
    }
}

impl<P: FeedProtocol> Owner<P> {
    /// Handles one inbound frame; `Some` ends the connection.
    pub(super) async fn frame(&mut self, frame: Message, sampler: &mut Sampler) -> Option<Ended> {
        let (kind, len) = match &frame {
            Message::Binary(b) => (FrameKind::Binary, b.len()),
            Message::Text(t) => (FrameKind::Text, t.len()),
            Message::Close(close) => {
                let code = close.as_ref().map(|c| u16::from(c.code));
                Span::current().record("disconnect_reason", "remote_close");
                return Some(Ended::Cause(
                    Cause::Disconnected(DisconnectReason::RemoteClose { code }),
                    None,
                ));
            }
            // Pings are answered by the WebSocket layer; pings and pongs only prove liveness.
            _ => return None,
        };
        let label = if kind == FrameKind::Binary {
            FrameLabel::Binary
        } else {
            FrameLabel::Text
        };
        metrics::record_ws_frame(P::FEED, label, len);
        let received_at = SystemTime::now();
        if self.capture_raw {
            let seq = self.next_seq();
            let raw = RawFrame {
                epoch: self.epoch,
                seq,
                received_at,
                kind,
                // Copied only when raw capture is on.
                bytes: match &frame {
                    Message::Binary(b) => b.to_vec(),
                    Message::Text(t) => t.as_bytes().to_vec(),
                    _ => Vec::new(),
                },
            };
            if let Err(t) = self.data(FeedEvent::Raw(raw)).await {
                return Some(Ended::Terminal(t));
            }
        }
        let span = spans::ws_frame(P::FEED, len);
        let mut out = Vec::new();
        span.in_scope(|| self.protocol.decode(&frame, &mut out));
        let mut packets = 0usize;
        let mut failed = false;
        for item in out {
            match item {
                Decoded::Data(value) => {
                    packets += 1;
                    let seq = self.next_seq();
                    self.last_data_seq = Some(seq);
                    let delivery = super::super::super::Delivery {
                        epoch: self.epoch,
                        seq,
                        received_at,
                        value,
                    };
                    if let Err(t) = self.data(FeedEvent::Data(delivery)).await {
                        return Some(Ended::Terminal(t));
                    }
                }
                Decoded::Error(error) => {
                    failed = true;
                    self.decode_error(&error, len, sampler);
                    let _seq = self.next_seq();
                    if let Err(t) = self.data(FeedEvent::DecodeError(error)).await {
                        return Some(Ended::Terminal(t));
                    }
                }
                Decoded::ServerDisconnect { code, ambiguous } => {
                    return Some(self.server_disconnect(code, ambiguous));
                }
            }
        }
        span.record("packets", packets);
        span.record(
            "result",
            if failed {
                if packets > 0 { "partial" } else { "error" }
            } else {
                "ok"
            },
        );
        None
    }

    fn decode_error(&self, error: &DecodeError, bytes: usize, sampler: &mut Sampler) {
        metrics::record_ws_decode_failure(P::FEED, decode_failure(error.kind));
        let Some(suppressed) = sampler.sample() else {
            return;
        };
        if error.kind == DecodeErrorKind::UnknownCode {
            emit!(
                Level::WARN,
                events::WS_UNKNOWN_PACKET,
                feed = P::FEED.as_str(),
                packet_code = error.packet_code,
                bytes,
                suppressed,
                "unknown packet code"
            );
        } else {
            emit!(
                Level::WARN,
                events::WS_DECODE_FAILED,
                feed = P::FEED.as_str(),
                kind = error.kind.as_str(),
                packet_code = error.packet_code,
                bytes,
                suppressed,
                "frame could not be decoded"
            );
        }
    }

    fn server_disconnect(&mut self, code: Option<u16>, ambiguous: Option<(u16, u32)>) -> Ended {
        let cause = Cause::ServerDisconnect(code);
        let terminal = matches!(dispose(&cause), Disposition::Terminal(_));
        let disposition = if terminal { "terminal" } else { "retry" };
        let code_field = code.map(u64::from);
        let ambiguous_field = ambiguous.map(|(a, b)| format!("{a}/{b}"));
        if terminal {
            emit!(
                Level::ERROR,
                events::WS_SERVER_DISCONNECT,
                feed = P::FEED.as_str(),
                epoch = self.epoch,
                code = code_field,
                ambiguous = ambiguous_field.as_deref(),
                disposition,
                "server disconnect"
            );
        } else {
            emit!(
                Level::WARN,
                events::WS_SERVER_DISCONNECT,
                feed = P::FEED.as_str(),
                epoch = self.epoch,
                code = code_field,
                ambiguous = ambiguous_field.as_deref(),
                disposition,
                "server disconnect"
            );
        }
        let span = Span::current();
        // An unreadable code is counted under the `other` label.
        metrics::record_ws_server_disconnect(P::FEED, code.unwrap_or(0));
        if let Some(code) = code {
            span.record("server_code", code);
            let known = DataErrorCode::from_u16(code);
            if let Err(t) = self.lifecycle(Lifecycle::ServerDisconnect {
                epoch: self.epoch,
                code,
                known,
            }) {
                return Ended::Terminal(t);
            }
        }
        Ended::Cause(cause, None)
    }

    /// A clean close: the protocol's disconnect message, a Close frame, then the peer's close,
    /// all within the shutdown timeout.
    pub(super) async fn close(&mut self, socket: &mut Socket) -> Ended {
        self.status.update(|s| s.state = FeedState::Stopping);
        let message = self.protocol.disconnect_message();
        let closing = async {
            if let Some(text) = message {
                socket.send(Message::Text(text.into())).await?;
            }
            socket.close(None).await?;
            while let Some(frame) = socket.next().await {
                if matches!(frame?, Message::Close(_)) {
                    break;
                }
            }
            Ok::<(), tungstenite::Error>(())
        };
        let finished = tokio::time::timeout(self.limits.shutdown_timeout, closing).await;
        if let Err(t) = self.lifecycle(Lifecycle::Disconnected {
            epoch: self.epoch,
            reason: DisconnectReason::Shutdown,
        }) {
            return Ended::Terminal(t);
        }
        match finished {
            Err(_) => Ended::Terminal(TerminalReason::ShutdownTimeout),
            // A peer that drops the connection instead of answering the close is still a
            // completed shutdown.
            Ok(_) => Ended::Stop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_failures_are_sampled_first_ten_then_every_thousandth() {
        let mut s = Sampler::default();
        let reported: Vec<(u64, u64)> = (1..=3000u64)
            .filter_map(|n| s.sample().map(|suppressed| (n, suppressed)))
            .collect();
        assert_eq!(reported.len(), 13);
        assert_eq!(
            reported[..10].iter().map(|r| r.1).collect::<Vec<_>>(),
            [0; 10]
        );
        assert_eq!(&reported[10..], &[(1000, 989), (2000, 999), (3000, 999)]);
    }
}
