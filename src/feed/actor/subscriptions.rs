//! Subscription bookkeeping: the canonical desired set and its revision.
//!
//! Each feed keeps one desired set of subscription keys (for the market feed,
//! `(instrument, mode)` pairs) and a revision. A command either changes the set whole or not at
//! all; a command that changes the set gets the next revision, and one that leaves it unchanged
//! keeps the current revision and sends nothing. Accepting a command is not a server
//! acknowledgement: Dhan sends none. A reconnect restores the whole set.

use std::collections::{BTreeMap, BTreeSet};

use super::super::TerminalReason;

/// A version of the desired subscription set: each accepted change increments it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(pub u64);

/// A change to a feed's desired subscriptions.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum SubscriptionCommand<S> {
    /// Desire each entry; an entry for an instrument already desired replaces it (its mode
    /// changes).
    Subscribe(Vec<S>),
    /// Stop desiring each entry's instrument (whatever its mode); instruments not desired are
    /// ignored.
    Unsubscribe(Vec<S>),
    /// Change the mode of instruments already desired; an instrument not desired is an error,
    /// never an implicit subscription.
    SetMode(Vec<S>),
    /// Replace the whole set.
    Replace(Vec<S>),
}

/// Why a subscription command was refused (nothing changed).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SubscriptionError {
    /// The command listed nothing.
    #[error("empty")]
    Empty,
    /// The result would exceed the per-connection capacity.
    #[error("capacity {max} exceeded")]
    CapacityExceeded {
        /// The capacity.
        max: usize,
    },
    /// `SetMode` named an instrument that is not desired.
    #[error("not subscribed")]
    NotSubscribed,
    /// The command named one instrument with two different modes.
    #[error("conflicting modes")]
    ConflictingModes,
    /// The feed does not accept this exchange segment.
    #[error("segment not allowed")]
    SegmentNotAllowed,
}

/// Why a feed command failed.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum CommandError {
    /// The command was refused; nothing changed.
    #[error(transparent)]
    Invalid(SubscriptionError),
    /// The command mailbox is full; retry later.
    #[error("mailbox full")]
    MailboxFull,
    /// The feed has ended.
    #[error("feed terminated: {0:?}")]
    Terminated(TerminalReason),
}

/// The canonical desired set and its revision.
#[allow(dead_code, reason = "owned by the feed owner task")]
#[derive(Clone, Debug)]
pub(crate) struct Desired<S> {
    set: BTreeSet<S>,
    revision: Revision,
}

#[allow(dead_code, reason = "used by the feed owner task")]
impl<S: Clone + Ord> Desired<S> {
    /// An empty set at revision 0.
    pub(crate) fn new() -> Self {
        Desired {
            set: BTreeSet::new(),
            revision: Revision(0),
        }
    }

    /// The desired entries.
    pub(crate) fn set(&self) -> &BTreeSet<S> {
        &self.set
    }

    /// The current revision.
    pub(crate) fn revision(&self) -> Revision {
        self.revision
    }

    /// Applies `command` whole or not at all, allowing at most `capacity` entries. `key` names
    /// an entry's instrument (entries with the same key are one subscription in different
    /// modes). Returns the new revision, or `None` if the set did not change.
    pub(crate) fn apply<K: Ord>(
        &mut self,
        command: &SubscriptionCommand<S>,
        capacity: usize,
        key: fn(&S) -> K,
    ) -> Result<Option<Revision>, SubscriptionError> {
        let entries = match command {
            SubscriptionCommand::Subscribe(e)
            | SubscriptionCommand::Unsubscribe(e)
            | SubscriptionCommand::SetMode(e)
            | SubscriptionCommand::Replace(e) => e,
        };
        if entries.is_empty() {
            return Err(SubscriptionError::Empty);
        }
        // One entry per instrument within the command.
        let mut listed: BTreeMap<K, &S> = BTreeMap::new();
        for e in entries {
            if let Some(previous) = listed.insert(key(e), e)
                && previous != e
                && !matches!(command, SubscriptionCommand::Unsubscribe(_))
            {
                return Err(SubscriptionError::ConflictingModes);
            }
        }
        let mut next: BTreeMap<K, S> = match command {
            SubscriptionCommand::Replace(_) => BTreeMap::new(),
            _ => self.set.iter().map(|s| (key(s), s.clone())).collect(),
        };
        match command {
            SubscriptionCommand::Subscribe(_) | SubscriptionCommand::Replace(_) => {
                for (k, e) in listed {
                    next.insert(k, e.clone());
                }
            }
            SubscriptionCommand::Unsubscribe(_) => {
                for k in listed.keys() {
                    next.remove(k);
                }
            }
            SubscriptionCommand::SetMode(_) => {
                for (k, e) in listed {
                    match next.get_mut(&k) {
                        Some(current) => *current = e.clone(),
                        None => return Err(SubscriptionError::NotSubscribed),
                    }
                }
            }
        }
        if next.len() > capacity {
            return Err(SubscriptionError::CapacityExceeded { max: capacity });
        }
        let next: BTreeSet<S> = next.into_values().collect();
        if next == self.set {
            return Ok(None);
        }
        self.set = next;
        // A u64 of accepted changes cannot overflow in practice; saturate rather than panic.
        self.revision = Revision(self.revision.0.saturating_add(1));
        Ok(Some(self.revision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(instrument, mode)`; the key is the instrument.
    type Sub = (u32, u8);

    fn key(s: &Sub) -> u32 {
        s.0
    }

    fn apply(
        d: &mut Desired<Sub>,
        cmd: SubscriptionCommand<Sub>,
    ) -> Result<Option<Revision>, SubscriptionError> {
        d.apply(&cmd, 3, key)
    }

    fn set(d: &Desired<Sub>) -> Vec<Sub> {
        d.set().iter().copied().collect()
    }

    #[test]
    fn subscribing_adds_and_changes_modes_under_new_revisions() {
        let mut d = Desired::new();
        assert_eq!(d.revision(), Revision(0));
        assert_eq!(
            apply(
                &mut d,
                SubscriptionCommand::Subscribe(vec![(1, 15), (2, 15)])
            ),
            Ok(Some(Revision(1)))
        );
        // Re-subscribing an instrument in another mode replaces its entry.
        assert_eq!(
            apply(&mut d, SubscriptionCommand::Subscribe(vec![(1, 21)])),
            Ok(Some(Revision(2)))
        );
        assert_eq!(set(&d), [(1, 21), (2, 15)]);
        // The same subscribe again changes nothing: same revision, nothing to send.
        assert_eq!(
            apply(&mut d, SubscriptionCommand::Subscribe(vec![(1, 21)])),
            Ok(None)
        );
        assert_eq!(d.revision(), Revision(2));
    }

    #[test]
    fn unsubscribing_ignores_modes_and_unknown_instruments() {
        let mut d = Desired::new();
        apply(
            &mut d,
            SubscriptionCommand::Subscribe(vec![(1, 15), (2, 17)]),
        )
        .unwrap();
        assert_eq!(
            apply(
                &mut d,
                SubscriptionCommand::Unsubscribe(vec![(2, 99), (7, 15)])
            ),
            Ok(Some(Revision(2)))
        );
        assert_eq!(set(&d), [(1, 15)]);
        assert_eq!(
            apply(&mut d, SubscriptionCommand::Unsubscribe(vec![(7, 15)])),
            Ok(None)
        );
    }

    #[test]
    fn set_mode_requires_a_subscription_and_is_all_or_nothing() {
        let mut d = Desired::new();
        apply(&mut d, SubscriptionCommand::Subscribe(vec![(1, 15)])).unwrap();
        assert_eq!(
            apply(&mut d, SubscriptionCommand::SetMode(vec![(1, 21), (2, 21)])),
            Err(SubscriptionError::NotSubscribed)
        );
        assert_eq!((set(&d), d.revision()), (vec![(1, 15)], Revision(1)));
        assert_eq!(
            apply(&mut d, SubscriptionCommand::SetMode(vec![(1, 21)])),
            Ok(Some(Revision(2)))
        );
        assert_eq!(set(&d), [(1, 21)]);
    }

    #[test]
    fn replace_swaps_the_whole_set() {
        let mut d = Desired::new();
        apply(
            &mut d,
            SubscriptionCommand::Subscribe(vec![(1, 15), (2, 15)]),
        )
        .unwrap();
        assert_eq!(
            apply(&mut d, SubscriptionCommand::Replace(vec![(3, 17)])),
            Ok(Some(Revision(2)))
        );
        assert_eq!(set(&d), [(3, 17)]);
        assert_eq!(
            apply(&mut d, SubscriptionCommand::Replace(vec![(3, 17)])),
            Ok(None)
        );
    }

    #[test]
    fn invalid_commands_change_nothing() {
        let mut d = Desired::new();
        apply(&mut d, SubscriptionCommand::Subscribe(vec![(1, 15)])).unwrap();
        for (cmd, err) in [
            (
                SubscriptionCommand::Subscribe(vec![]),
                SubscriptionError::Empty,
            ),
            (
                SubscriptionCommand::Unsubscribe(vec![]),
                SubscriptionError::Empty,
            ),
            (
                SubscriptionCommand::SetMode(vec![]),
                SubscriptionError::Empty,
            ),
            (
                SubscriptionCommand::Replace(vec![]),
                SubscriptionError::Empty,
            ),
            (
                SubscriptionCommand::Subscribe(vec![(2, 15), (2, 21)]),
                SubscriptionError::ConflictingModes,
            ),
            (
                SubscriptionCommand::Replace(vec![(2, 15), (2, 17)]),
                SubscriptionError::ConflictingModes,
            ),
            (
                SubscriptionCommand::Subscribe(vec![(2, 15), (3, 15), (4, 15)]),
                SubscriptionError::CapacityExceeded { max: 3 },
            ),
        ] {
            assert_eq!(apply(&mut d, cmd), Err(err.clone()), "{err:?}");
            assert_eq!((set(&d), d.revision()), (vec![(1, 15)], Revision(1)));
        }
        // A duplicate identical entry is not a conflict.
        assert_eq!(
            apply(
                &mut d,
                SubscriptionCommand::Subscribe(vec![(2, 15), (2, 15)])
            ),
            Ok(Some(Revision(2)))
        );
        // Exactly at capacity is fine.
        assert_eq!(
            apply(&mut d, SubscriptionCommand::Subscribe(vec![(3, 15)])),
            Ok(Some(Revision(3)))
        );
    }

    #[test]
    fn capacity_one_refuses_a_second_instrument() {
        let mut d = Desired::new();
        assert_eq!(
            d.apply(
                &SubscriptionCommand::Subscribe(vec![(1, 15), (2, 15)]),
                1,
                key
            ),
            Err(SubscriptionError::CapacityExceeded { max: 1 })
        );
        assert_eq!(
            d.apply(&SubscriptionCommand::Subscribe(vec![(1, 15)]), 1, key),
            Ok(Some(Revision(1)))
        );
    }

    #[test]
    fn command_errors_display() {
        assert_eq!(
            CommandError::Invalid(SubscriptionError::CapacityExceeded { max: 5000 }).to_string(),
            "capacity 5000 exceeded"
        );
        assert_eq!(CommandError::MailboxFull.to_string(), "mailbox full");
    }
}
