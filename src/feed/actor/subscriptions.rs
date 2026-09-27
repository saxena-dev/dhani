//! Subscription bookkeeping.

/// A version of the desired subscription set: each accepted command increments it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(pub u64);
