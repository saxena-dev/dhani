//! `FeedHandle<S>` and `FeedTask`.

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::actor::{CommandError, Envelope, Revision, StatusReader, Stop, SubscriptionCommand};
use super::{FeedError, FeedStatus, TaskOutcome, TerminalReason};
use crate::obs::spans;

/// Controls a running feed: subscription commands, status and shutdown. Cheap to clone; every
/// clone controls the same feed. When every handle is dropped the feed ends.
pub struct FeedHandle<S> {
    tx: mpsc::Sender<Envelope<S>>,
    status: StatusReader,
    stop: Arc<Stop>,
}

impl<S> Clone for FeedHandle<S> {
    fn clone(&self) -> Self {
        FeedHandle {
            tx: self.tx.clone(),
            status: self.status.clone(),
            stop: Arc::clone(&self.stop),
        }
    }
}

impl<S> std::fmt::Debug for FeedHandle<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeedHandle").finish_non_exhaustive()
    }
}

fn command_label<S>(command: &SubscriptionCommand<S>) -> &'static str {
    match command {
        SubscriptionCommand::Subscribe(_) => "subscribe",
        SubscriptionCommand::Unsubscribe(_) => "unsubscribe",
        SubscriptionCommand::SetMode(_) => "set_mode",
        SubscriptionCommand::Replace(_) => "replace",
    }
}

impl<S: Send + 'static> FeedHandle<S> {
    pub(crate) fn new(
        tx: mpsc::Sender<Envelope<S>>,
        status: StatusReader,
        stop: Arc<Stop>,
    ) -> Self {
        FeedHandle { tx, status, stop }
    }

    fn terminated(&self) -> CommandError {
        CommandError::Terminated(
            self.status
                .snapshot()
                .terminal
                .unwrap_or(TerminalReason::Shutdown),
        )
    }

    /// Sends a subscription command. `Ok(revision)` means the feed accepted the command (the
    /// server sends no acknowledgement). A full mailbox is an immediate
    /// [`CommandError::MailboxFull`].
    pub async fn command(&self, command: SubscriptionCommand<S>) -> Result<Revision, CommandError> {
        // Opened in the caller's context and entered by the owner while applying.
        let span = spans::ws_command(command_label(&command));
        let (reply, answer) = oneshot::channel();
        let envelope = Envelope {
            command,
            reply,
            span,
        };
        match self.tx.try_send(envelope) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => return Err(CommandError::MailboxFull),
            Err(mpsc::error::TrySendError::Closed(_)) => return Err(self.terminated()),
        }
        answer.await.unwrap_or_else(|_| Err(self.terminated()))
    }

    /// The current status; never waits.
    pub fn status(&self) -> FeedStatus {
        self.status.snapshot()
    }

    /// The version of the current status, for [`changed`](FeedHandle::changed).
    pub fn status_version(&self) -> u64 {
        self.status.version()
    }

    /// Waits for a status newer than version `since` (several changes coalesce into one).
    pub async fn changed(&self, since: u64) -> FeedStatus {
        self.status.changed(since).await.1
    }

    /// Shuts the feed down and waits for it to end. `Ok` after a clean shutdown; otherwise the
    /// reason the feed ended.
    pub async fn shutdown(&self) -> Result<(), FeedError> {
        self.stop.request();
        let mut version = self.status.version();
        loop {
            let status = self.status.snapshot();
            if let Some(reason) = status.terminal {
                return match reason {
                    TerminalReason::Shutdown => Ok(()),
                    other => Err(FeedError(other)),
                };
            }
            let (next, _) = self.status.changed(version).await;
            if next == version {
                // The owner is gone without a terminal reason (its task was cancelled).
                return Err(FeedError(TerminalReason::Panicked));
            }
            version = next;
        }
    }
}

/// The feed's task. [`join`](FeedTask::join) waits for its outcome; dropping it does nothing.
#[derive(Debug)]
pub struct FeedTask {
    handle: JoinHandle<TaskOutcome>,
}

impl FeedTask {
    pub(crate) fn new(handle: JoinHandle<TaskOutcome>) -> Self {
        FeedTask { handle }
    }

    /// Waits for the feed to end.
    pub async fn join(self) -> TaskOutcome {
        self.handle.await.unwrap_or(TaskOutcome::Panicked)
    }
}

const _: () = {
    fn send_sync_clone<T: Send + Sync + Clone>() {}
    fn send_unpin<T: Send + Unpin>() {}
    #[allow(dead_code, reason = "compile-time assertions only")]
    fn assertions<S: Send + 'static, T: Send + 'static>() {
        send_sync_clone::<FeedHandle<S>>();
        send_unpin::<super::FeedEvents<T>>();
        send_unpin::<FeedTask>();
    }
};
