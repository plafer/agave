//! Serde mirrors of the spec types that are compared with agave.
//!
//! Quint sum types are encoded in ITF as `{ "tag": ..., "value": ... }`, which serde reads with
//! adjacent tagging. Record fields that are not mirrored here are ignored when deserializing.

use {
    super::mapping::SpecBlock,
    serde::Deserialize,
    std::{
        collections::{BTreeMap, BTreeSet},
        fmt::{Debug, Write},
    },
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

impl Message {
    /// The name of the spec's constructor.
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Message::NotarVoteMsg(_) => "NotarVoteMsg",
            Message::NotarFallBackVoteMsg(_) => "NotarFallBackVoteMsg",
            Message::SkipVoteMsg(_) => "SkipVoteMsg",
            Message::SkipFallbackVoteMsg(_) => "SkipFallbackVoteMsg",
            Message::FinalVoteMsg(_) => "FinalVoteMsg",
        }
    }
}

/// Mirror of the spec's `NetworkMsg`.
#[derive(Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) struct NetworkMsg {
    pub(super) sender: String,
    pub(super) msg: Message,
}

/// Mirror of the spec's `SlotObject`, including the two variants added by agave patch 5.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(tag = "tag", content = "value")]
pub(super) enum SlotObject {
    ParentReady(i64),
    Voted,
    VotedNotar(i64),
    BlockNotarized(i64),
    ItsOver,
    BadWindow,
    VotedNotarFallback(i64),
    VotedSkipFallback,
}

/// Mirror of the spec's `LocalState`: per-slot lists, indexed by spec slot.
#[derive(Deserialize, PartialEq, Eq, Debug)]
pub(super) struct LocalState {
    /// Blocks received for each slot that could not be voted on yet, in arrival order.
    #[serde(rename = "pendingBlocks")]
    pub(super) pending_blocks: Vec<Vec<SpecBlock>>,
    pub(super) state: Vec<BTreeSet<SlotObject>>,
}

/// The part of the spec's `Environment` that is compared with agave after every step.
///
/// `activeTimeouts`, `ch`, and `counter` are not compared (see the `mbt` module doc).
#[derive(Deserialize, Eq, Debug)]
pub(super) struct SpecState {
    /// The local state of every benevolent process.
    pub(super) system: BTreeMap<String, LocalState>,
    /// Every vote broadcast by a benevolent process since `init`.
    #[serde(rename = "msgBuffer")]
    pub(super) msg_buffer: BTreeSet<NetworkMsg>,
}

impl PartialEq for SpecState {
    /// Also prints what differs, because quint-connect only prints its own diff when it is built
    /// with `QUINT_VERBOSE` set. quint-connect evaluates `spec_state != driver_state`, so `self`
    /// is the spec's state and `other` is agave's.
    fn eq(&self, other: &Self) -> bool {
        let equal = self.system == other.system && self.msg_buffer == other.msg_buffer;
        if !equal {
            let mut out = String::from("state differs between the spec and agave:\n");
            set_diff(&mut out, "msgBuffer", &self.msg_buffer, &other.msg_buffer);
            system_diff(&mut out, &self.system, &other.system);
            eprint!("{out}");
        }
        equal
    }
}

/// Describes, per process and slot, what differs between the spec's `system` and agave's.
fn system_diff(
    out: &mut String,
    spec: &BTreeMap<String, LocalState>,
    agave: &BTreeMap<String, LocalState>,
) {
    let processes: BTreeSet<&String> = spec.keys().chain(agave.keys()).collect();
    for v in processes {
        let (Some(spec), Some(agave)) = (spec.get(v), agave.get(v)) else {
            let _ = writeln!(out, "  process {v} is missing on one side");
            continue;
        };
        if spec.state.len() != agave.state.len()
            || spec.pending_blocks.len() != agave.pending_blocks.len()
        {
            let _ = writeln!(
                out,
                "  {v}: the spec keeps {} slots of state and {} of pending blocks, agave {} and {}",
                spec.state.len(),
                spec.pending_blocks.len(),
                agave.state.len(),
                agave.pending_blocks.len(),
            );
        }
        for (slot, (spec, agave)) in spec.state.iter().zip(&agave.state).enumerate() {
            set_diff(out, &format!("{v}.state[{slot}]"), spec, agave);
        }
        for (slot, (spec, agave)) in spec
            .pending_blocks
            .iter()
            .zip(&agave.pending_blocks)
            .enumerate()
        {
            if spec != agave {
                let _ = writeln!(
                    out,
                    "  {v}.pendingBlocks[{slot}]: spec {spec:?}, agave {agave:?}"
                );
            }
        }
    }
}

/// Describes the elements that are only in the spec's set or only in agave's.
fn set_diff<T: Ord + Debug>(out: &mut String, name: &str, spec: &BTreeSet<T>, agave: &BTreeSet<T>) {
    for (label, only) in [
        ("only in the spec", spec.difference(agave)),
        ("only in agave", agave.difference(spec)),
    ] {
        for item in only {
            let _ = writeln!(out, "  {name}: {label}: {item:?}");
        }
    }
}
