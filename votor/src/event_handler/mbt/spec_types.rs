//! Serde mirrors of the spec types that are compared with agave.
//!
//! Quint sum types are encoded in ITF as `{ "tag": ..., "value": ... }`, which serde reads with
//! adjacent tagging. Record fields that are not mirrored here are ignored when deserializing.

use {
    super::mapping::SpecBlock,
    anyhow::{Context, Result, bail},
    itf::value::{Record, Value},
    serde::Deserialize,
    std::{
        collections::{BTreeMap, BTreeSet},
        fmt::{Debug, Write},
    },
};

/// A spec action together with the nondet picks it uses: one variant per action of the spec's
/// `step` and `init`, named and with picks named as in the spec.
#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "tag", content = "value")]
pub(super) enum SpecAction {
    /// `init`, or `initGenerated` (agave patch 8), which picks the trace's blocks.
    #[serde(rename = "init", alias = "initGenerated")]
    Init { blocks: BTreeSet<SpecBlock> },
    #[serde(rename = "receiveBlock")]
    ReceiveBlock { v: String, block: SpecBlock },
    #[serde(rename = "fireTimeoutEvent")]
    FireTimeout { v: String, slot: i64 },
    #[serde(rename = "blockNotarizedAction")]
    BlockNotarized { v: String, b: SpecBlock },
    #[serde(rename = "parentReadyAction")]
    ParentReady { v: String, slot: i64, b: SpecBlock },
    #[serde(rename = "safeToNotarAction")]
    SafeToNotar { v: String, b: SpecBlock },
    #[serde(rename = "safeToSkipAction")]
    SafeToSkip { v: String, slot: i64 },
}

impl SpecAction {
    /// Reads the action from an ITF state's `mbt::actionTaken` and `mbt::nondetPicks`.
    ///
    /// `nondet_picks` is a record with one `Option` per nondet pick of the spec: the picks of
    /// this step are `Some`, and the others `None`. The picks the action does not use
    /// (`slotCoin`, `timeoutCoin`, `kind`, and so on) are ignored.
    pub(super) fn from_mbt(action_taken: &str, nondet_picks: Value) -> Result<Self> {
        let Value::Record(picks) = nondet_picks else {
            bail!("`mbt::nondetPicks` is not a record: {nondet_picks:?}");
        };
        let picks: Record = picks
            .into_iter()
            .filter_map(|(name, pick)| some_value(pick).map(|pick| (name, pick)))
            .collect();
        let mut action = Record::new();
        action.insert("tag".to_string(), Value::String(action_taken.to_string()));
        action.insert("value".to_string(), Value::Record(picks));
        Self::deserialize(Value::Record(action))
            .with_context(|| format!("cannot read spec action {action_taken:?} and its picks"))
    }

    /// The name of the spec action, with `init` for both initial actions.
    pub(super) fn name(&self) -> &'static str {
        match self {
            SpecAction::Init { .. } => "init",
            SpecAction::ReceiveBlock { .. } => "receiveBlock",
            SpecAction::FireTimeout { .. } => "fireTimeoutEvent",
            SpecAction::BlockNotarized { .. } => "blockNotarizedAction",
            SpecAction::ParentReady { .. } => "parentReadyAction",
            SpecAction::SafeToNotar { .. } => "safeToNotarAction",
            SpecAction::SafeToSkip { .. } => "safeToSkipAction",
        }
    }
}

/// The value of an ITF `Option`: `Some` for `{ tag: "Some", value }`, `None` for
/// `{ tag: "None", .. }`. Any other value is returned as it is, as quint-connect does.
fn some_value(value: Value) -> Option<Value> {
    match value {
        Value::Record(mut record) => match record.get("tag") {
            Some(Value::String(tag)) if tag == "Some" => record.remove("value"),
            Some(Value::String(tag)) if tag == "None" => None,
            _ => Some(Value::Record(record)),
        },
        other => Some(other),
    }
}

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

/// Mirror of the spec's `Certificate`.
// Variant names must match the spec's constructors.
#[allow(clippy::enum_variant_names)]
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(tag = "tag", content = "value")]
pub(super) enum Certificate {
    FastFinalizationCertificate(BlockRef),
    NotarizationCertificate(BlockRef),
    NotarFallbackCertificate(BlockRef),
    SkipCertificate(i64),
    FinalizationCertificate(i64),
}

/// Mirror of the spec's `PoolView` (agave patch 13): what a pool holding every message knows.
#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
pub(super) struct PoolView {
    /// Every certificate whose votes are among the messages.
    pub(super) certificates: BTreeSet<Certificate>,
    /// Every `(slot, parent hash)` for which `ParentReady` holds, including the initial
    /// `(0, -1)`.
    #[serde(rename = "parentReady")]
    pub(super) parent_ready: BTreeSet<(i64, i64)>,
    /// For every benevolent process, the blocks for which `SafeToNotar` holds.
    #[serde(rename = "safeToNotar")]
    pub(super) safe_to_notar: BTreeMap<String, BTreeSet<BlockRef>>,
    /// For every benevolent process, the slots for which `SafeToSkip` holds.
    #[serde(rename = "safeToSkip")]
    pub(super) safe_to_skip: BTreeMap<String, BTreeSet<i64>>,
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
    /// The spec's pool view (agave patch 13), which every instance has. On agave's side, the view
    /// that every node's pool shares, or `None` in an instance without pools. `None` is not
    /// compared.
    pub(super) pool: Option<PoolView>,
}

impl PartialEq for SpecState {
    /// Also prints what differs, because quint-connect only prints its own diff when it is built
    /// with `QUINT_VERBOSE` set. quint-connect evaluates `spec_state != driver_state`, so `self`
    /// is the spec's state and `other` is agave's.
    fn eq(&self, other: &Self) -> bool {
        let pools = match (&self.pool, &other.pool) {
            (Some(spec), Some(agave)) => Some((spec, agave)),
            _ => None,
        };
        let equal = self.system == other.system
            && self.msg_buffer == other.msg_buffer
            && pools.is_none_or(|(spec, agave)| spec == agave);
        if !equal {
            let mut out = String::from("state differs between the spec and agave:\n");
            set_diff(&mut out, "msgBuffer", &self.msg_buffer, &other.msg_buffer);
            if let Some((spec, agave)) = pools {
                set_diff(
                    &mut out,
                    "pool.certificates",
                    &spec.certificates,
                    &agave.certificates,
                );
                set_diff(
                    &mut out,
                    "pool.parentReady",
                    &spec.parent_ready,
                    &agave.parent_ready,
                );
                map_diff(
                    &mut out,
                    "pool.safeToNotar",
                    &spec.safe_to_notar,
                    &agave.safe_to_notar,
                );
                map_diff(
                    &mut out,
                    "pool.safeToSkip",
                    &spec.safe_to_skip,
                    &agave.safe_to_skip,
                );
            }
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

/// Describes, per process, the elements that are only in the spec's set or only in agave's.
fn map_diff<T: Ord + Debug>(
    out: &mut String,
    name: &str,
    spec: &BTreeMap<String, BTreeSet<T>>,
    agave: &BTreeMap<String, BTreeSet<T>>,
) {
    let processes: BTreeSet<&String> = spec.keys().chain(agave.keys()).collect();
    for v in processes {
        let (Some(spec), Some(agave)) = (spec.get(v), agave.get(v)) else {
            let _ = writeln!(out, "  {name}: process {v} is missing on one side");
            continue;
        };
        set_diff(out, &format!("{name}[{v}]"), spec, agave);
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
