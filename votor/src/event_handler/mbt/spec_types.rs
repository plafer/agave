//! Serde mirrors of the spec types that are compared with agave.
//!
//! Quint sum types are encoded in ITF as `{ "tag": ..., "value": ... }`, which serde reads with
//! adjacent tagging. Record fields that are not mirrored here are ignored when deserializing.

use {
    serde::Deserialize,
    std::{collections::BTreeSet, fmt::Debug},
};

/// Mirror of the spec's `BlockReference`.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) struct BlockRef {
    pub(super) slot: i64,
    pub(super) hash: i64,
}

/// Mirror of the spec's `Message`.
// Variant names must match the spec's constructors.
#[allow(clippy::enum_variant_names)]
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(tag = "tag", content = "value")]
pub(super) enum Message {
    NotarVoteMsg(BlockRef),
    NotarFallBackVoteMsg(BlockRef),
    SkipVoteMsg(i64),
    SkipFallbackVoteMsg(i64),
    FinalVoteMsg(i64),
}

/// Mirror of the spec's `NetworkMsg`.
#[derive(Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) struct NetworkMsg {
    pub(super) sender: String,
    pub(super) msg: Message,
}

/// The part of the spec's `Environment` that is compared with agave after every step.
///
/// `system`, `activeTimeouts`, `ch`, and `counter` are not compared yet.
#[derive(Deserialize, Eq, Debug)]
pub(super) struct SpecState {
    /// Every vote broadcast by a benevolent process since `init`.
    #[serde(rename = "msgBuffer")]
    pub(super) msg_buffer: BTreeSet<NetworkMsg>,
}

impl PartialEq for SpecState {
    /// Also prints what differs, because quint-connect only prints its own diff when it is built
    /// with `QUINT_VERBOSE` set. quint-connect evaluates `spec_state != driver_state`, so `self`
    /// is the spec's state and `other` is agave's.
    fn eq(&self, other: &Self) -> bool {
        let equal = self.msg_buffer == other.msg_buffer;
        if !equal {
            eprintln!(
                "{}",
                set_diff("msgBuffer", &self.msg_buffer, &other.msg_buffer)
            );
        }
        equal
    }
}

/// Describes the elements that are only in the spec's set or only in agave's.
fn set_diff<T: Ord + Debug>(name: &str, spec: &BTreeSet<T>, agave: &BTreeSet<T>) -> String {
    let mut out = format!("{name} differs between the spec and agave:\n");
    for (label, only) in [
        ("only in the spec", spec.difference(agave)),
        ("only in agave", agave.difference(spec)),
    ] {
        for item in only {
            out.push_str(&format!("  {label}: {item:?}\n"));
        }
    }
    out
}
