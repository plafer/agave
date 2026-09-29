//! Model-based tests of the event handler against the Alpenglow Quint specification.
//!
//! [quint-connect] runs `quint run --mbt` on the vendored spec in `spec/` and replays every
//! generated trace step against agave. Each benevolent spec process gets its own
//! [`EventHandlerTestContext`], and each spec action becomes exactly one [`VotorEvent`] passed to
//! [`EventHandler::handle_event`] for the process that took the step. The driver has no copy of
//! the instance's blocks: each trace's `init` step picks the trace's block set, and the driver
//! builds a bank for each block from that pick. In the `agave_gen` instance that set is a block
//! tree drawn anew for every trace (spec patch 8). See `mapping.rs` for how spec slots, hashes,
//! and blocks map onto agave values.
//!
//! # Running
//!
//! The tests are `#[ignore]`d because they need the `quint` CLI on `PATH`:
//!
//! ```text
//! npm install -g @informalsystems/quint
//! cargo nextest run -p agave-votor --run-ignored ignored-only mbt
//! QUINT_SEED=0x... cargo nextest run -p agave-votor --run-ignored ignored-only mbt  # reproduce
//! QUINT_VERBOSE=1 cargo nextest run -p agave-votor --run-ignored ignored-only mbt --no-capture
//! ```
//!
//! # What is checked
//!
//! Only the event handler is under test. Replaying a trace checks that:
//!
//! - `handle_event` neither fails nor panics (for example on a `VoteHistory` equivocation
//!   assert), and only emits `BLSOp::PushVote`;
//! - a timer is set for the slot of every `ParentReady`;
//! - no node pushes a vote it already pushed since `init`. The spec's `msgBuffer` is a set, so a
//!   repeated vote would otherwise go unnoticed; with spec patches 5 and 6 a correct spec process
//!   never broadcasts the same message twice either.
//!
//! After every step, the driver's projection of agave's state must equal the spec's environment
//! variable `s`, restricted to:
//!
//! - `msgBuffer`: the votes pushed by all nodes since `init`;
//! - `system`: for every benevolent process and every spec slot, the `VoteHistory` flags (voted,
//!   voted-notar, block-notarized, parent-ready, its-over, bad-window, notar-fallback voted,
//!   skip-fallback voted) and `LocalContext::pending_blocks` in arrival order.
//!
//! # Trusted, not checked
//!
//! - The consensus pool. Certificates, parent-ready, safe-to-notar, and safe-to-skip are decided
//!   by the spec's environment over its message soup (which includes every possible Byzantine
//!   vote) and injected as events. Agave's pool-side behavior, such as deferring intra-window
//!   `SafeToNotar` until the parent is verified and emitting each derived event once, is not
//!   exercised.
//! - The timer manager. The spec's `activeTimeouts` is a set of slots that may fire in any order,
//!   while agave arms one two-phase timer per window that fires in slot order by wall-clock
//!   time. `activeTimeouts` is not compared. Instead `fireTimeoutEvent` injects `Timeout`
//!   directly, and each `ParentReady` must leave a timer set for its slot.
//! - The spec's `ch` (finalized chain) and `counter` (trace bound), which are environment
//!   bookkeeping.
//! - Agave-only events, which are never sent: `FirstShred`, `TimeoutCrashedLeader`,
//!   `BlockNotarFallback`, `ProduceWindow`, `Finalized`, `Standstill`, and `SetIdentity`. With no
//!   `Finalized` event the root stays at genesis, so rooting and root-based pruning are never
//!   reached.
//! - Agave state without a spec counterpart: `VoteHistory`'s `skipped` set (only visible through
//!   `bad_window`), `votes_cast`, and root; `LocalContext`'s finalized blocks, received shreds,
//!   and standstill slot; commitment, repair, metrics, reward, and persistence side effects (vote
//!   history storage is `NullVoteHistoryStorage`).
//! - Agave's `slot == 1` special case for the first leader window, which the slot offset keeps
//!   out of reach (see `mapping.rs`).
//!
//! # Spec patches
//!
//! The vendored spec is patched where agave's behavior is the intended one, and extended with the
//! agave-specific `agave_window` and `agave_gen` instances. `spec/README.md` describes every
//! patch:
//!
//! 1. Pending blocks are a per-slot list, tried in arrival order, as in agave.
//! 2. `parentReadyAction` requires skip certificates only for the slots strictly between the
//!    parent and the ready slot, and records `ParentReady` at the ready slot.
//! 3. Per-process lists cover every window that has an alive slot.
//! 4. The initial `ParentReady` starts the timeouts of its whole window.
//! 5. `VotedNotarFallback(hash)` and `VotedSkipFallback` record fallback votes, which are cast at
//!    most once per block or slot, as in agave.
//! 6. `tryFinal` requires `not(ItsOver)`, as agave's `try_final` does.
//! 7. The block set is part of the environment, and `init` exposes it as a nondet pick.
//! 8. `initGenerated` draws a block tree per trace, for the `agave_gen` instance.
//!
//! [quint-connect]: https://github.com/quint-co/quint-connect

mod mapping;
mod spec_types;

use {
    self::{
        mapping::{
            Banks, GENESIS_HASH, OFFSET, SpecBlock, agave_slot, block_ref, hash_of, spec_hash,
            spec_slot, to_spec_block, to_spec_message,
        },
        spec_types::{LocalState, NetworkMsg, SlotObject, SpecState},
    },
    super::{
        EventHandler,
        test_context::{EventHandlerTestContext, SharedFixtures, setup_node},
    },
    crate::{
        event::{CompletedBlock, VotorEvent},
        vote_history_storage::NullVoteHistoryStorage,
        voting_service::BLSOp,
    },
    agave_votor_messages::consensus_message::Block,
    anyhow::{anyhow, bail, ensure},
    quint_connect::{Config, Driver, Path, Result, State, Step, quint_run, switch},
    std::{
        any::Any,
        collections::{BTreeMap, BTreeSet},
        marker::PhantomData,
        panic::{AssertUnwindSafe, catch_unwind},
        sync::Arc,
    },
};

type Node = EventHandlerTestContext<NullVoteHistoryStorage>;

/// A spec instance module: which ITF variable holds the environment, and the instance's
/// constants that the driver needs, copied from `spec/statemachine.qnt`.
trait Instance {
    /// Path to the environment variable `s` in each ITF state.
    const STATE_PATH: Path;
    /// `(process id, stake, is benevolent)` for every spec process, with stakes copied from the
    /// instance's `power` table. Byzantine processes get a stake but no node.
    const PROCESSES: &'static [(&'static str, u64, bool)];
    /// The instance's `numSlots` (agave patch 3): the length of every per-slot list in a
    /// process's `LocalState`.
    const NUM_SLOTS: usize;
}

/// The `some_byz` instance: five equal-stake correct processes and one Byzantine process.
struct SomeByz;

impl Instance for SomeByz {
    const STATE_PATH: Path = &["some_byz::consensus::s"];
    const PROCESSES: &'static [(&'static str, u64, bool)] = &[
        ("v1", 1, true),
        ("v2", 1, true),
        ("v3", 1, true),
        ("v4", 1, true),
        ("v5", 1, true),
        ("b1", 1, false),
    ];
    const NUM_SLOTS: usize = 4;
}

/// The `some_byz_vp` instance: two correct processes and one Byzantine process, with different
/// stakes.
struct SomeByzVp;

impl Instance for SomeByzVp {
    const STATE_PATH: Path = &["some_byz_vp::consensus::s"];
    const PROCESSES: &'static [(&'static str, u64, bool)] =
        &[("v1", 3, true), ("v2", 2, true), ("b1", 1, false)];
    const NUM_SLOTS: usize = 4;
}

/// The agave-specific `agave_window` instance, whose slot 4 starts a second leader window, so
/// that traces can replay `parentReadyAction`.
struct AgaveWindow;

impl Instance for AgaveWindow {
    const STATE_PATH: Path = &["agave_window::consensus::s"];
    const PROCESSES: &'static [(&'static str, u64, bool)] =
        &[("v1", 3, true), ("v2", 2, true), ("b1", 1, false)];
    const NUM_SLOTS: usize = 8;
}

/// The agave-specific `agave_gen` instance, run with `initGenerated`: every trace draws its own
/// block tree over two leader windows (spec patch 8).
struct AgaveGen;

impl Instance for AgaveGen {
    const STATE_PATH: Path = &["agave_gen::consensus::s"];
    const PROCESSES: &'static [(&'static str, u64, bool)] = &[
        ("v1", 3, true),
        ("v2", 2, true),
        ("v3", 2, true),
        ("b1", 1, false),
    ];
    const NUM_SLOTS: usize = 8;
}

/// Replays spec traces against one event handler per benevolent spec process.
struct VotorMbtDriver<I: Instance> {
    /// Kept alive for the banks and blockstore the nodes share.
    _fixtures: SharedFixtures,
    banks: Banks,
    /// Benevolent processes only, keyed by spec process id.
    nodes: BTreeMap<String, Node>,
    /// Every vote pushed by a node since `init`: agave's counterpart of the spec's `msgBuffer`.
    msg_buffer: BTreeSet<NetworkMsg>,
    stats: ReplayStats,
    _instance: PhantomData<I>,
}

/// Counters reported when the driver is dropped, to show what the traces exercised.
#[derive(Default)]
struct ReplayStats {
    actions: BTreeMap<String, usize>,
    votes: usize,
    traces: usize,
    /// Number of traces whose block tree has each shape.
    tree_shapes: BTreeMap<TreeShape, usize>,
}

impl ReplayStats {
    fn record_tree(&mut self, blocks: &BTreeSet<SpecBlock>, num_slots: usize) {
        self.traces = self.traces.saturating_add(1);
        for shape in TreeShape::of(blocks, num_slots) {
            let count = self.tree_shapes.entry(shape).or_default();
            *count = count.saturating_add(1);
        }
    }
}

/// Shapes of a trace's block tree that [`ReplayStats`] counts. They are computed from the
/// `blocks` pick of `init`, not from the generator's own picks, so that the driver does not
/// depend on the generator's vocabulary.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum TreeShape {
    /// Two blocks in one slot: an equivocation, and a fork if their parents differ.
    Equivocation,
    /// A parent that is neither genesis nor one of the trace's blocks.
    PhantomParent,
    /// A parent block more than one slot back.
    FarParent,
    /// The genesis parent for a block after slot 0.
    LateGenesisParent,
    /// A spec slot without a block.
    EmptySlot,
}

impl TreeShape {
    const ALL: [TreeShape; 5] = [
        TreeShape::Equivocation,
        TreeShape::PhantomParent,
        TreeShape::FarParent,
        TreeShape::LateGenesisParent,
        TreeShape::EmptySlot,
    ];

    /// The shapes of the tree `blocks`, whose slots are in `0..num_slots`.
    fn of(blocks: &BTreeSet<SpecBlock>, num_slots: usize) -> BTreeSet<TreeShape> {
        let slot_of: BTreeMap<i64, i64> = blocks.iter().map(|b| (b.hash, b.slot)).collect();
        let mut per_slot: BTreeMap<i64, usize> = BTreeMap::new();
        let mut shapes = BTreeSet::new();
        for block in blocks {
            let count = per_slot.entry(block.slot).or_default();
            *count = count.saturating_add(1);
            if *count > 1 {
                shapes.insert(TreeShape::Equivocation);
            }
            let shape = if block.parent == GENESIS_HASH {
                (block.slot > 0).then_some(TreeShape::LateGenesisParent)
            } else {
                match slot_of.get(&block.parent) {
                    None => Some(TreeShape::PhantomParent),
                    Some(&parent_slot) => {
                        (parent_slot < block.slot.saturating_sub(1)).then_some(TreeShape::FarParent)
                    }
                }
            };
            shapes.extend(shape);
        }
        let empty_slot = (0..num_slots).any(|s| {
            let s = i64::try_from(s).expect("few slots");
            !per_slot.contains_key(&s)
        });
        if empty_slot {
            shapes.insert(TreeShape::EmptySlot);
        }
        shapes
    }
}

impl<I: Instance> VotorMbtDriver<I> {
    fn new() -> Self {
        let stakes = I::PROCESSES.iter().map(|&(_, stake, _)| stake).collect();
        let fixtures = SharedFixtures::new(stakes);
        let nodes = I::PROCESSES
            .iter()
            .enumerate()
            .filter(|(_, (_, _, benevolent))| *benevolent)
            .map(|(index, (id, _, _))| {
                let storage = Arc::new(NullVoteHistoryStorage::default());
                (id.to_string(), setup_node(&fixtures, index, storage))
            })
            .collect();
        let genesis = fixtures.bank_forks.read().unwrap().root_bank();
        Self {
            _fixtures: fixtures,
            banks: Banks::new(genesis),
            nodes,
            msg_buffer: BTreeSet::new(),
            stats: ReplayStats::default(),
            _instance: PhantomData,
        }
    }

    /// Spec `init`: the trace's blocks are `blocks`, and every benevolent process starts with
    /// `ParentReady(-1)` at slot 0.
    fn init(&mut self, blocks: BTreeSet<SpecBlock>) -> Result {
        self.stats.record_tree(&blocks, I::NUM_SLOTS);
        self.banks.reset(blocks)?;
        self.msg_buffer.clear();
        let ids: Vec<String> = self.nodes.keys().cloned().collect();
        for id in ids {
            self.node(&id)?.reset();
            self.parent_ready_event(&id, OFFSET, Block::default())?;
        }
        Ok(())
    }

    fn receive_block(&mut self, v: &str, block: SpecBlock) -> Result {
        let bank = self.banks.bank_for(&block)?;
        let slot = agave_slot(block.slot);
        self.handle(v, VotorEvent::Block(CompletedBlock { slot, bank }))
    }

    fn fire_timeout(&mut self, v: &str, slot: i64) -> Result {
        self.handle(v, VotorEvent::Timeout(agave_slot(slot)))
    }

    fn block_notarized(&mut self, v: &str, b: SpecBlock) -> Result {
        self.handle(v, VotorEvent::BlockNotarized(block_ref(b.slot, b.hash)))
    }

    fn parent_ready(&mut self, v: &str, slot: i64, b: SpecBlock) -> Result {
        self.parent_ready_event(v, agave_slot(slot), block_ref(b.slot, b.hash))
    }

    fn safe_to_notar(&mut self, v: &str, b: SpecBlock) -> Result {
        self.handle(v, VotorEvent::SafeToNotar(block_ref(b.slot, b.hash)))
    }

    fn safe_to_skip(&mut self, v: &str, slot: i64) -> Result {
        self.handle(v, VotorEvent::SafeToSkip(agave_slot(slot)))
    }

    /// Sends `ParentReady` and checks that it armed the timer for `slot`, which is agave's
    /// counterpart of the spec's `ScheduleEventTimeout` outputs.
    fn parent_ready_event(&mut self, v: &str, slot: u64, parent_block: Block) -> Result {
        self.handle(v, VotorEvent::ParentReady { slot, parent_block })?;
        ensure!(
            self.node(v)?.timer_manager.read().is_timeout_set(slot),
            "{v}: no timeout set for slot {slot} after ParentReady"
        );
        Ok(())
    }

    /// Passes `event` to the event handler of process `v` and records the votes it pushes in
    /// `msg_buffer`. Errors and panics are both turned into errors so that quint-connect reports
    /// the seed that reproduces them.
    fn handle(&mut self, v: &str, event: VotorEvent) -> Result {
        let description = format!("{v}: {event:?}");
        let node = self.node(v)?;
        let result = catch_unwind(AssertUnwindSafe(|| {
            EventHandler::handle_event(
                event,
                &node.timer_manager,
                &node.shared_context,
                &mut node.voting_context,
                &node.root_context,
                &mut node.local_context,
            )
        }));
        let ops = match result {
            Ok(Ok(ops)) => ops,
            Ok(Err(err)) => bail!("handle_event failed on {description}: {err:?}"),
            Err(payload) => bail!(
                "handle_event panicked on {description}: {}",
                panic_message(&*payload)
            ),
        };
        for op in ops {
            match op {
                BLSOp::PushVote { vote } => {
                    self.stats.votes = self.stats.votes.saturating_add(1);
                    let msg = NetworkMsg {
                        sender: v.to_string(),
                        msg: to_spec_message(&vote.vote)?,
                    };
                    // `msgBuffer` is a set, so a repeated vote is invisible to the state
                    // comparison. With agave patches 5 and 6 a correct spec process never
                    // broadcasts the same message twice, and neither should agave.
                    ensure!(
                        !self.msg_buffer.contains(&msg),
                        "{description}: pushed {msg:?} again"
                    );
                    self.msg_buffer.insert(msg);
                }
                BLSOp::PushCertificates { .. }
                | BLSOp::RefreshVotes { .. }
                | BLSOp::RefreshCertificates { .. } => {
                    bail!("unexpected BLSOp on {description}: {op:?}")
                }
            }
        }
        Ok(())
    }

    fn node(&mut self, v: &str) -> Result<&mut Node> {
        self.nodes
            .get_mut(v)
            .ok_or_else(|| anyhow!("no node for spec process {v:?}"))
    }
}

impl<I: Instance> State<VotorMbtDriver<I>> for SpecState {
    fn from_driver(driver: &VotorMbtDriver<I>) -> Result<Self> {
        let system = driver
            .nodes
            .iter()
            .map(|(v, node)| {
                let local_state = project_local_state::<I>(v, node, driver.banks.blocks())?;
                Ok((v.clone(), local_state))
            })
            .collect::<Result<_>>()?;
        Ok(SpecState {
            system,
            msg_buffer: driver.msg_buffer.clone(),
        })
    }
}

/// Projects a node's `VoteHistory` and pending blocks onto the spec's `LocalState`, for spec
/// slots `0..I::NUM_SLOTS`.
///
/// `VoteHistory` is queried through its accessors, so the flags that carry a hash are probed with
/// every hash of the trace's `blocks` and their parents, plus the genesis hash. Every hash agave
/// sees comes from one of these, since the driver builds every event from the trace's blocks.
fn project_local_state<I: Instance>(
    v: &str,
    node: &Node,
    blocks: &BTreeSet<SpecBlock>,
) -> Result<LocalState> {
    let vote_history = &node.voting_context.vote_history;
    let hashes: BTreeSet<i64> = blocks
        .iter()
        .flat_map(|b| [b.hash, b.parent])
        .chain([GENESIS_HASH])
        .collect();
    // The agave block a spec `ParentReady(h)` refers to. Only the trace's blocks (and the genesis
    // block) can become parent-ready in the spec.
    let parent_ref = |h: i64| {
        if h == GENESIS_HASH {
            Some(Block::default())
        } else {
            blocks
                .iter()
                .find(|b| b.hash == h)
                .map(|b| block_ref(b.slot, h))
        }
    };

    let mut state = Vec::with_capacity(I::NUM_SLOTS);
    for s in 0..I::NUM_SLOTS {
        let s = i64::try_from(s).expect("few slots");
        let a = agave_slot(s);
        let mut objects = BTreeSet::new();
        if vote_history.voted(a) {
            objects.insert(SlotObject::Voted);
        }
        if let Some(hash) = vote_history.voted_notar(a) {
            objects.insert(SlotObject::VotedNotar(spec_hash(&hash)?));
        }
        if vote_history.its_over(a) {
            objects.insert(SlotObject::ItsOver);
        }
        if vote_history.bad_window(a) {
            objects.insert(SlotObject::BadWindow);
        }
        if vote_history.voted_skip_fallback(a) {
            objects.insert(SlotObject::VotedSkipFallback);
        }
        for &h in &hashes {
            let block = Block {
                slot: a,
                block_id: hash_of(h),
            };
            if vote_history.is_block_notarized(&block) {
                objects.insert(SlotObject::BlockNotarized(h));
            }
            if vote_history.voted_notar_fallback(a, hash_of(h)) {
                objects.insert(SlotObject::VotedNotarFallback(h));
            }
            if parent_ref(h).is_some_and(|parent| vote_history.is_parent_ready(a, &parent)) {
                objects.insert(SlotObject::ParentReady(h));
            }
        }
        state.push(objects);
    }

    let mut pending_blocks = vec![Vec::new(); I::NUM_SLOTS];
    for (&slot, blocks) in &node.local_context.pending_blocks {
        let pending = usize::try_from(spec_slot(slot)?)
            .ok()
            .and_then(|s| pending_blocks.get_mut(s))
            .ok_or_else(|| {
                anyhow!("{v}: pending blocks at agave slot {slot}, beyond the spec's")
            })?;
        *pending = blocks
            .iter()
            .map(|(block, parent)| to_spec_block(block, parent))
            .collect::<Result<_>>()?;
    }

    Ok(LocalState {
        pending_blocks,
        state,
    })
}

impl<I: Instance> Driver for VotorMbtDriver<I> {
    type State = SpecState;

    fn config() -> Config {
        Config {
            state: I::STATE_PATH,
            nondet: &[],
        }
    }

    fn step(&mut self, step: &Step) -> Result {
        let count = self
            .stats
            .actions
            .entry(step.action_taken.clone())
            .or_default();
        *count = count.saturating_add(1);
        switch!(step {
            init(blocks: BTreeSet<SpecBlock>) => self.init(blocks)?,
            initGenerated(blocks: BTreeSet<SpecBlock>) => self.init(blocks)?,
            receiveBlock(v: String, block: SpecBlock) => self.receive_block(&v, block)?,
            fireTimeoutEvent(v: String, slot: i64) => self.fire_timeout(&v, slot)?,
            blockNotarizedAction(v: String, b: SpecBlock) => self.block_notarized(&v, b)?,
            parentReadyAction(v: String, slot: i64, b: SpecBlock) => {
                self.parent_ready(&v, slot, b)?
            },
            safeToNotarAction(v: String, b: SpecBlock) => self.safe_to_notar(&v, b)?,
            safeToSkipAction(v: String, slot: i64) => self.safe_to_skip(&v, slot)?,
        })
    }
}

impl<I: Instance> Drop for VotorMbtDriver<I> {
    fn drop(&mut self) {
        let ReplayStats {
            actions,
            votes,
            traces,
            tree_shapes,
        } = &self.stats;
        let tree_shapes: Vec<String> = TreeShape::ALL
            .iter()
            .map(|shape| {
                let count = tree_shapes.get(shape).copied().unwrap_or_default();
                let percent = count.saturating_mul(100).checked_div(*traces).unwrap_or(0);
                format!("{shape:?} {count} ({percent}%)")
            })
            .collect();
        eprintln!(
            "votor MBT replay of {}: actions {actions:?}, votes pushed {votes}, traces {traces}, \
             traces whose block tree has: {}",
            I::STATE_PATH.join("."),
            tree_shapes.join(", "),
        );
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>")
}

/// Requires the `quint` CLI on `PATH` (`npm install -g @informalsystems/quint`).
///
/// Run with: `cargo nextest run -p agave-votor --features agave-unstable-api --run-ignored ignored-only mbt`
#[ignore]
#[quint_run(
    spec = "src/event_handler/mbt/spec/statemachine.qnt",
    main = "some_byz",
    max_samples = 100
)]
fn mbt_votor_some_byz() -> impl Driver {
    VotorMbtDriver::<SomeByz>::new()
}

/// Requires the `quint` CLI on `PATH` (`npm install -g @informalsystems/quint`).
///
/// Run with: `cargo nextest run -p agave-votor --features agave-unstable-api --run-ignored ignored-only mbt`
#[ignore]
#[quint_run(
    spec = "src/event_handler/mbt/spec/statemachine.qnt",
    main = "some_byz_vp",
    max_samples = 100
)]
fn mbt_votor_some_byz_vp() -> impl Driver {
    VotorMbtDriver::<SomeByzVp>::new()
}

/// Requires the `quint` CLI on `PATH` (`npm install -g @informalsystems/quint`).
///
/// Run with: `cargo nextest run -p agave-votor --features agave-unstable-api --run-ignored ignored-only mbt`
///
/// Only about one trace in 13 reaches `parentReadyAction`, hence the larger sample.
#[ignore]
#[quint_run(
    spec = "src/event_handler/mbt/spec/statemachine.qnt",
    main = "agave_window",
    max_samples = 200
)]
fn mbt_votor_agave_window() -> impl Driver {
    VotorMbtDriver::<AgaveWindow>::new()
}

/// Requires the `quint` CLI on `PATH` (`npm install -g @informalsystems/quint`).
///
/// Run with: `cargo nextest run -p agave-votor --features agave-unstable-api --run-ignored ignored-only mbt`
///
/// Every trace starts from its own generated block tree (spec patch 8).
#[ignore]
#[quint_run(
    spec = "src/event_handler/mbt/spec/statemachine.qnt",
    main = "agave_gen",
    init = "initGenerated",
    max_samples = 200
)]
fn mbt_votor_agave_gen() -> impl Driver {
    VotorMbtDriver::<AgaveGen>::new()
}
