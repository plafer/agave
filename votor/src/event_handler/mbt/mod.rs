//! Model-based tests of the event handler, and of the consensus pool, against the Alpenglow Quint
//! specification.
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
//! In the `agave_pool` instance, every node also runs its own `ConsensusPool` (see `pool.rs`),
//! which receives every vote any node pushes. There, a spec action that delivers
//! `BlockNotarized` or `ParentReady` passes on the event the node's pool emitted, instead of
//! building it from the action's picks, and after every step the pools must hold what the spec's
//! pool view (spec patch 13) holds.
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
//! The event handler is under test in every instance, and the consensus pool in `agave_pool`.
//! Replaying a trace checks that:
//!
//! - `handle_event` neither fails nor panics (for example on a `VoteHistory` equivocation
//!   assert), and only emits `BLSOp::PushVote`;
//! - a timer is set for the slot of every `ParentReady`;
//! - no node pushes a vote it already pushed since `init`. The spec's `msgBuffer` is a set, so a
//!   repeated vote would otherwise go unnoticed; with spec patches 5 and 6 a correct spec process
//!   never broadcasts the same message twice either;
//! - in `agave_pool`, every `blockNotarizedAction` and `parentReadyAction` finds the matching
//!   `BlockNotarized` or `ParentReady` among the events the node's pool emitted, so the pool is at
//!   least as live as the spec for these two events. The event is passed to the event handler the
//!   first time the spec delivers it, and not again, as in agave; with spec patches 5 and 6 a
//!   repeated input leaves the spec's compared state unchanged. The votes a node sends its own
//!   pool must also be the votes it pushes;
//! - in `agave_pool`, all pools hold the same certificates and have emitted the same
//!   `ParentReady` events, since each of them received every vote.
//!
//! After every step, the driver's projection of agave's state must equal the spec's environment
//! variable `s`, restricted to:
//!
//! - `msgBuffer`: the votes pushed by all nodes since `init`;
//! - `system`: for every benevolent process and every spec slot, the `VoteHistory` flags (voted,
//!   voted-notar, block-notarized, parent-ready, its-over, bad-window, notar-fallback voted,
//!   skip-fallback voted) and `LocalContext::pending_blocks` in arrival order;
//! - `pool`, in `agave_pool` only: the certificates the pools hold, and the `(slot, parent)`
//!   pairs of every `ParentReady` they emitted, plus the initial one. The spec's view is the
//!   certificates and parent-ready pairs over all messages, so the pools must be exactly as live
//!   as the spec for both. A pool builds only the strongest of the certificates a block's
//!   notarize votes make, so its `Notarize` and `FinalizeFast` certificates also count as the
//!   block's notar-fallback certificate, as agave's `ParentReadyTracker` counts them. Agave
//!   does not count `FinalizeFast` as `Notarize`, and neither does the projection.
//!
//! # Trusted, not checked
//!
//! - The consensus pool, outside `agave_pool`. Certificates, parent-ready, safe-to-notar, and
//!   safe-to-skip are decided by the spec's environment over its message soup (which includes
//!   every possible Byzantine vote) and injected as events.
//! - In `agave_pool`: `SafeToNotar` and `SafeToSkip`, which still come from the spec's picks (the
//!   pools emit them too, and they are counted but not used), so agave's deferral of
//!   intra-window `SafeToNotar` until the parent is verified is not exercised; `BlockNotarFallback`
//!   and `Finalized`, which the pools emit but are never dispatched (only the certificates they
//!   follow from are compared); anything the pools hold past the last spec slot; and Byzantine
//!   voters, which `agave_pool` does not have (see `pool.rs`).
//! - The timer manager. The spec's `activeTimeouts` is a set of slots that may fire in any order,
//!   while agave arms one two-phase timer per window that fires in slot order by wall-clock
//!   time. `activeTimeouts` is not compared. Instead `fireTimeoutEvent` injects `Timeout`
//!   directly, and each `ParentReady` must leave a timer set for its slot.
//! - The spec's `ch` (finalized chain) and `counter` (step count), which are environment
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
//! # Glue between the pools
//!
//! In `agave_pool`, the driver stands in for the parts of agave around the pools:
//!
//! - Vote delivery. Every vote a node pushes reaches every pool before the next step, one vote at
//!   a time in the order it was cast: the node's own pool gets the `VoteMessage` from the node's
//!   own-vote channel, as `PoolVote::Own`, and every other pool gets it as a
//!   `PoolVote::External` `VoteAggregate`, without bls-sigverify's signature checks, conflict
//!   filter, or batching.
//! - The initial `ParentReady`. Every pool starts with spec slot 0 (agave slot `OFFSET`)
//!   parent-ready for genesis, as its event handler does, and the driver sends that
//!   `ParentReady` to the event handler itself, as the consensus pool service does. The pool
//!   view counts this pair as emitted, and the spec's view always has it.
//! - A fresh pool per node and trace, rooted at the genesis bank, which stays the root, so the
//!   pools never prune.
//!
//! One divergence is avoided by the choice of stakes rather than resolved. When a single vote
//! takes a block's notarize votes from below 60% to 80% or more, agave's pool forms only the
//! fast-finalization certificate, and emits `Finalized(block, true)` but no `BlockNotarized`,
//! while the spec's `blockNotarizedAction` is enabled. With `agave_gen`'s stakes (3, 2, 2, 1) this
//! happens when v2 and v3 notarize a block before v1, so `agave_pool` uses 3, 2, 2, 2, where no
//! single vote can do it. In production, bls-sigverify batches votes into aggregates, so such a
//! jump is common there. The pool view comparison would also catch it, as a
//! `NotarizationCertificate` only the spec has.
//!
//! # Spec patches
//!
//! The vendored spec is patched where agave's behavior is the intended one, and extended with the
//! agave-specific `agave_window`, `agave_gen`, and `agave_pool` instances. `spec/README.md`
//! describes every patch:
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
//! 9. `step` favors steps that make progress: each process's lowest unvoted slot, and fewer
//!    timeouts and repeated block deliveries. Traces are as long as each test's `max_steps`.
//! 10. The Byzantine soup leaves out votes for generated hashes of other slots, which no
//!     generated block has, to make `agave_gen` steps faster.
//! 11. `isDescendant` walks slots in ascending order, which `quint run`'s Rust backend does not
//!     guarantee for a fold over a set. Only the `safety` invariant reads it.
//! 12. The `agave_pool` instance: `agave_gen` without Byzantine processes.
//! 13. The environment has a pool view: the certificates and parent-ready pairs over all
//!     messages, recomputed after every step in instances without Byzantine processes. No
//!     action reads it.
//! 14. A window start whose earlier slots are all skip-certified is parent-ready for genesis,
//!     as in agave's `ParentReadyTracker`.
//! 17. `safeToSkipCondition` requires that the process sent no skip vote in the slot, as the
//!     paper and agave do. Numbers 15 and 16 are reserved for later pool patches.
//! 18. A slot with only skip-fallback votes can be skip-certified for `parentReadyAction`, as
//!     it already can for `isCertified`.
//!
//! [quint-connect]: https://github.com/quint-co/quint-connect

mod mapping;
mod pool;
mod spec_types;

use {
    self::{
        mapping::{
            Banks, GENESIS_HASH, OFFSET, SpecBlock, agave_slot, block_ref, hash_of, spec_hash,
            spec_slot, to_spec_block, to_spec_message,
        },
        pool::{NodePool, PoolEvent},
        spec_types::{LocalState, Message, NetworkMsg, PoolView, SlotObject, SpecState},
    },
    super::{
        EventHandler,
        test_context::{EventHandlerTestContext, SharedFixtures, setup_node},
    },
    crate::{
        consensus_pool_service::PoolVote,
        event::{CompletedBlock, VotorEvent},
        tests::new_vote_aggregate,
        vote_history_storage::NullVoteHistoryStorage,
        voting_service::BLSOp,
    },
    agave_votor_messages::{
        certificate::CertificateType,
        consensus_message::{Block, VoteMessage},
    },
    anyhow::{anyhow, bail, ensure},
    quint_connect::{Config, Driver, Path, Result, State, Step, quint_run, switch},
    solana_runtime::bank::Bank,
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
    /// Whether every node runs a consensus pool, from which `BlockNotarized` and `ParentReady`
    /// are taken instead of from the spec's picks. Only for instances without Byzantine
    /// processes (see `pool.rs`).
    const WITH_POOL: bool;
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
    const WITH_POOL: bool = false;
}

/// The `some_byz_vp` instance: two correct processes and one Byzantine process, with different
/// stakes.
struct SomeByzVp;

impl Instance for SomeByzVp {
    const STATE_PATH: Path = &["some_byz_vp::consensus::s"];
    const PROCESSES: &'static [(&'static str, u64, bool)] =
        &[("v1", 3, true), ("v2", 2, true), ("b1", 1, false)];
    const NUM_SLOTS: usize = 4;
    const WITH_POOL: bool = false;
}

/// The agave-specific `agave_window` instance, whose slot 4 starts a second leader window, so
/// that traces can replay `parentReadyAction`.
struct AgaveWindow;

impl Instance for AgaveWindow {
    const STATE_PATH: Path = &["agave_window::consensus::s"];
    const PROCESSES: &'static [(&'static str, u64, bool)] =
        &[("v1", 3, true), ("v2", 2, true), ("b1", 1, false)];
    const NUM_SLOTS: usize = 8;
    const WITH_POOL: bool = false;
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
    const WITH_POOL: bool = false;
}

/// The agave-specific `agave_pool` instance, run with `initGenerated`: `agave_gen` with its
/// Byzantine process turned into the correct `v4`, of stake 2 (spec patch 12). Every node runs a
/// consensus pool.
struct AgavePool;

impl Instance for AgavePool {
    const STATE_PATH: Path = &["agave_pool::consensus::s"];
    const PROCESSES: &'static [(&'static str, u64, bool)] = &[
        ("v1", 3, true),
        ("v2", 2, true),
        ("v3", 2, true),
        ("v4", 2, true),
    ];
    const NUM_SLOTS: usize = 8;
    const WITH_POOL: bool = true;
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
    /// The genesis bank, which stays the root bank of every pool.
    genesis: Arc<Bank>,
    /// If `I::WITH_POOL`, the consensus pool of every node, keyed like `nodes` and rebuilt by
    /// every `init`. Empty otherwise.
    pools: BTreeMap<String, NodePool>,
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
    /// Number of traces that take each action, or push each kind of vote, at least once.
    traces_with: BTreeMap<String, usize>,
    /// The actions taken and the kinds of vote pushed so far in the current trace.
    current_trace: BTreeSet<String>,
    /// Number of derived events of each kind that the pools emitted, summed over the pools.
    pool_emitted: BTreeMap<&'static str, usize>,
    /// Number of pool events of each kind that a spec action passed to an event handler.
    pool_dispatched: BTreeMap<&'static str, usize>,
    /// Number of spec actions that asked for a pool event that was already dispatched.
    pool_repeated: BTreeMap<&'static str, usize>,
}

impl ReplayStats {
    /// Starts a new trace whose block tree is `blocks`.
    fn start_trace(&mut self, blocks: &BTreeSet<SpecBlock>, num_slots: usize) {
        self.finish_trace();
        self.traces = self.traces.saturating_add(1);
        for shape in TreeShape::of(blocks, num_slots) {
            let count = self.tree_shapes.entry(shape).or_default();
            *count = count.saturating_add(1);
        }
    }

    /// Adds the current trace's actions and vote kinds to `traces_with`.
    fn finish_trace(&mut self) {
        for name in std::mem::take(&mut self.current_trace) {
            let count = self.traces_with.entry(name).or_default();
            *count = count.saturating_add(1);
        }
    }

    fn record_action(&mut self, action: &str) {
        let count = self.actions.entry(action.to_string()).or_default();
        *count = count.saturating_add(1);
        self.current_trace.insert(action.to_string());
    }

    fn record_vote(&mut self, msg: &Message) {
        self.votes = self.votes.saturating_add(1);
        self.current_trace.insert(msg.kind().to_string());
    }

    fn record_pool_emitted(&mut self, event: &PoolEvent) {
        increment(&mut self.pool_emitted, event.kind());
    }

    /// Records a spec action that asked for the pool event `event`, which was passed to the event
    /// handler if `dispatched`, and had been passed before otherwise.
    fn record_pool_dispatch(&mut self, event: &PoolEvent, dispatched: bool) {
        if dispatched {
            increment(&mut self.pool_dispatched, event.kind());
            self.current_trace
                .insert(format!("dispatched {}", event.kind()));
        } else {
            increment(&mut self.pool_repeated, event.kind());
        }
    }

    /// Records the kinds of certificate that some pool holds at the end of the current trace.
    fn record_pool_certificates<'a>(
        &mut self,
        certificates: impl IntoIterator<Item = &'a CertificateType>,
    ) {
        for certificate in certificates {
            let kind = match certificate {
                CertificateType::Finalize(_) => "Finalize",
                CertificateType::FinalizeFast(_) => "FinalizeFast",
                CertificateType::Notarize(_) => "Notarize",
                CertificateType::NotarizeFallback(_) => "NotarizeFallback",
                CertificateType::Skip(_) => "Skip",
                CertificateType::Genesis(_) => "Genesis",
            };
            self.current_trace.insert(format!("certificate {kind}"));
        }
    }
}

fn increment(counts: &mut BTreeMap<&'static str, usize>, key: &'static str) {
    let count = counts.entry(key).or_default();
    *count = count.saturating_add(1);
}

/// `count` of `total` as a whole percentage.
fn percent(count: usize, total: usize) -> usize {
    count.saturating_mul(100).checked_div(total).unwrap_or(0)
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
            banks: Banks::new(genesis.clone()),
            nodes,
            msg_buffer: BTreeSet::new(),
            genesis,
            pools: BTreeMap::new(),
            stats: ReplayStats::default(),
            _instance: PhantomData,
        }
    }

    /// Spec `init`: the trace's blocks are `blocks`, and every benevolent process starts with
    /// `ParentReady(-1)` at slot 0.
    ///
    /// With `I::WITH_POOL`, every node also gets a fresh consensus pool (a `ConsensusPool` cannot
    /// be reset). The pools start with the same parent ready, which the driver sends to the event
    /// handlers itself, as the consensus pool service does.
    fn init(&mut self, blocks: BTreeSet<SpecBlock>) -> Result {
        self.stats
            .record_pool_certificates(self.pools.values().flat_map(NodePool::certificate_types));
        self.stats.start_trace(&blocks, I::NUM_SLOTS);
        self.banks.reset(blocks)?;
        self.msg_buffer.clear();
        self.pools.clear();
        if I::WITH_POOL {
            for (id, node) in &self.nodes {
                let pool = NodePool::new(node.cluster_info.clone(), &self.genesis);
                self.pools.insert(id.clone(), pool);
            }
        }
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
        let block = block_ref(b.slot, b.hash);
        if !I::WITH_POOL {
            return self.handle(v, VotorEvent::BlockNotarized(block));
        }
        let event = PoolEvent::BlockNotarized(block);
        if let Some(event) = self.dispatch_pool_event("blockNotarizedAction", v, event)? {
            self.handle(v, event)?;
        }
        Ok(())
    }

    fn parent_ready(&mut self, v: &str, slot: i64, b: SpecBlock) -> Result {
        let slot = agave_slot(slot);
        let parent = block_ref(b.slot, b.hash);
        if !I::WITH_POOL {
            return self.parent_ready_event(v, slot, parent);
        }
        let event = PoolEvent::ParentReady { slot, parent };
        if self
            .dispatch_pool_event("parentReadyAction", v, event)?
            .is_some()
        {
            self.parent_ready_event(v, slot, parent)?;
        }
        Ok(())
    }

    /// For a spec action that delivers the derived event `event` to process `v`, returns the
    /// event for `v`'s event handler: `v`'s pool must have emitted it, and it is returned the
    /// first time only. The spec may deliver the same derived event again, while agave delivers
    /// it once. With spec patches 5 and 6 a repeated input leaves the spec's state unchanged, so
    /// not passing it on again is checked by the state comparison.
    fn dispatch_pool_event(
        &mut self,
        action: &str,
        v: &str,
        event: PoolEvent,
    ) -> Result<Option<VotorEvent>> {
        let pool = self
            .pools
            .get_mut(v)
            .ok_or_else(|| anyhow!("no pool for spec process {v:?}"))?;
        let dispatch = pool
            .take_for_dispatch(&event)
            .map_err(|err| anyhow!("spec fired {action} for {v}, but {v}'s pool {err}"))?;
        self.stats.record_pool_dispatch(&event, dispatch.is_some());
        Ok(dispatch)
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
        let mut pushed = Vec::new();
        for op in ops {
            match op {
                BLSOp::PushVote { vote } => {
                    let msg = NetworkMsg {
                        sender: v.to_string(),
                        msg: to_spec_message(&vote.vote)?,
                    };
                    self.stats.record_vote(&msg.msg);
                    // `msgBuffer` is a set, so a repeated vote is invisible to the state
                    // comparison. With agave patches 5 and 6 a correct spec process never
                    // broadcasts the same message twice, and neither should agave.
                    ensure!(
                        !self.msg_buffer.contains(&msg),
                        "{description}: pushed {msg:?} again"
                    );
                    self.msg_buffer.insert(msg);
                    pushed.push(vote);
                }
                BLSOp::PushCertificates { .. }
                | BLSOp::RefreshVotes { .. }
                | BLSOp::RefreshCertificates { .. } => {
                    bail!("unexpected BLSOp on {description}: {op:?}")
                }
            }
        }
        if I::WITH_POOL {
            self.deliver_votes(v, &description, pushed)?;
        }
        Ok(())
    }

    /// Delivers the votes `v` pushed in one step to every pool, one vote at a time and in the
    /// order they were cast, as the spec's global message soup does. `v`'s own pool gets each
    /// vote as the `VoteMessage` the event handler sent on its own-vote channel, and every other
    /// pool gets it as a `VoteAggregate`, as bls-sigverify forwards a vote received from the
    /// network.
    fn deliver_votes(
        &mut self,
        v: &str,
        description: &str,
        pushed: Vec<Arc<VoteMessage>>,
    ) -> Result {
        let own: Vec<VoteMessage> = self.node(v)?.own_vote_receiver.try_iter().collect();
        ensure!(
            own.len() == pushed.len() && own.iter().zip(&pushed).all(|(o, p)| o.vote == p.vote),
            "{description}: sent {own:?} to its own pool but pushed {pushed:?}"
        );
        for (own, pushed) in own.into_iter().zip(pushed) {
            for (u, pool) in &mut self.pools {
                let vote = if u == v {
                    PoolVote::Own(own.clone())
                } else {
                    PoolVote::External(new_vote_aggregate(&self.genesis, (*pushed).clone()))
                };
                for event in pool.add_votes(&self.genesis, vec![vote])? {
                    self.stats.record_pool_emitted(&event);
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
        let pool = if I::WITH_POOL {
            Some(project_pools::<I>(&driver.pools)?)
        } else {
            None
        };
        Ok(SpecState {
            system,
            msg_buffer: driver.msg_buffer.clone(),
            pool,
        })
    }
}

/// Projects the pools onto the spec's pool view, for spec slots `0..I::NUM_SLOTS`. Every pool
/// receives every vote, so all of them must have the same view, as the spec has a single one.
fn project_pools<I: Instance>(pools: &BTreeMap<String, NodePool>) -> Result<PoolView> {
    let mut pools = pools.iter();
    let (first, pool) = pools
        .next()
        .ok_or_else(|| anyhow!("no pools in an instance with pools"))?;
    let view = pool.view(I::NUM_SLOTS)?;
    for (v, pool) in pools {
        let other = pool.view(I::NUM_SLOTS)?;
        ensure!(
            other == view,
            "the pools of {first} and {v} differ, although both received every vote: {first} has \
             {view:?}, {v} has {other:?}"
        );
    }
    Ok(view)
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
        self.replay(step)?;
        // After the step, so that an `init` step counts towards the trace it starts.
        self.stats.record_action(&step.action_taken);
        Ok(())
    }
}

impl<I: Instance> VotorMbtDriver<I> {
    /// Replays one trace step: the spec action becomes one event for the process that took it.
    fn replay(&mut self, step: &Step) -> Result {
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
        self.stats
            .record_pool_certificates(self.pools.values().flat_map(NodePool::certificate_types));
        self.stats.finish_trace();
        let ReplayStats {
            actions,
            votes,
            traces,
            tree_shapes,
            traces_with,
            current_trace: _,
            pool_emitted,
            pool_dispatched,
            pool_repeated,
        } = &self.stats;
        let tree_shapes: Vec<String> = TreeShape::ALL
            .iter()
            .map(|shape| {
                let count = tree_shapes.get(shape).copied().unwrap_or_default();
                format!("{shape:?} {count} ({}%)", percent(count, *traces))
            })
            .collect();
        let traces_with: Vec<String> = traces_with
            .iter()
            .map(|(name, &count)| format!("{name} {count} ({}%)", percent(count, *traces)))
            .collect();
        let pools = if I::WITH_POOL {
            format!(
                ", pool events emitted {pool_emitted:?}, dispatched {pool_dispatched:?}, asked \
                 for again {pool_repeated:?}"
            )
        } else {
            String::new()
        };
        eprintln!(
            "votor MBT replay of {}: actions {actions:?}, votes pushed {votes}, traces {traces}, \
             traces whose block tree has: {}, traces with each action, vote, dispatched pool \
             event, or pool certificate: {}{pools}",
            I::STATE_PATH.join("."),
            tree_shapes.join(", "),
            traces_with.join(", "),
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
    max_samples = 100,
    max_steps = 20
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
    max_samples = 100,
    max_steps = 20
)]
fn mbt_votor_some_byz_vp() -> impl Driver {
    VotorMbtDriver::<SomeByzVp>::new()
}

/// Requires the `quint` CLI on `PATH` (`npm install -g @informalsystems/quint`).
///
/// Run with: `cargo nextest run -p agave-votor --features agave-unstable-api --run-ignored ignored-only mbt`
#[ignore]
#[quint_run(
    spec = "src/event_handler/mbt/spec/statemachine.qnt",
    main = "agave_window",
    max_samples = 100,
    max_steps = 20
)]
fn mbt_votor_agave_window() -> impl Driver {
    VotorMbtDriver::<AgaveWindow>::new()
}

/// Requires the `quint` CLI on `PATH` (`npm install -g @informalsystems/quint`).
///
/// Run with: `cargo nextest run -p agave-votor --features agave-unstable-api --run-ignored ignored-only mbt`
///
/// Every trace starts from its own generated block tree (spec patch 8). Its traces are longer
/// than the other instances', so that they get past the first leader window.
#[ignore]
#[quint_run(
    spec = "src/event_handler/mbt/spec/statemachine.qnt",
    main = "agave_gen",
    init = "initGenerated",
    max_samples = 180,
    max_steps = 60
)]
fn mbt_votor_agave_gen() -> impl Driver {
    VotorMbtDriver::<AgaveGen>::new()
}

/// Requires the `quint` CLI on `PATH` (`npm install -g @informalsystems/quint`).
///
/// Run with: `cargo nextest run -p agave-votor --features agave-unstable-api --run-ignored ignored-only mbt`
///
/// Like `mbt_votor_agave_gen`, without Byzantine processes, and with a consensus pool per node
/// that decides which `BlockNotarized` and `ParentReady` events exist (spec patch 12).
#[ignore]
#[quint_run(
    spec = "src/event_handler/mbt/spec/statemachine.qnt",
    main = "agave_pool",
    init = "initGenerated",
    max_samples = 150,
    max_steps = 60
)]
fn mbt_votor_agave_pool() -> impl Driver {
    VotorMbtDriver::<AgavePool>::new()
}
