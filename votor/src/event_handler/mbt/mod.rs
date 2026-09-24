//! Model-based tests of the event handler against the Alpenglow Quint specification.
//!
//! [quint-connect] runs `quint run --mbt` on the vendored spec in `spec/` and replays every
//! generated trace step against agave. Each benevolent spec process gets its own
//! [`EventHandlerTestContext`], and each spec action becomes exactly one [`VotorEvent`] passed to
//! [`EventHandler::handle_event`] for the process that took the step. See `mapping.rs` for how
//! spec slots, hashes, and blocks map onto agave values.
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
//! # Scope
//!
//! Only the event handler is under test. The environment's decisions (certificates, parent-ready,
//! safe-to-notar, safe-to-skip, and which timeouts fire) come from the trace, so the consensus
//! pool and the timer manager are trusted, not checked. Agave-only events (`FirstShred`,
//! `TimeoutCrashedLeader`, `BlockNotarFallback`, `ProduceWindow`, `Finalized`, `Standstill`,
//! `SetIdentity`) are never sent.
//!
//! Replaying a trace currently checks that `handle_event` neither fails nor panics (for example
//! on a `VoteHistory` equivocation assert), that it only emits `BLSOp::PushVote`, and that a
//! timer is set after every `ParentReady`. The node state is not yet compared with the spec's.
//!
//! [quint-connect]: https://github.com/quint-co/quint-connect

mod mapping;

use {
    self::mapping::{Banks, OFFSET, SpecBlock, agave_slot, block_ref},
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
    quint_connect::{Config, Driver, Path, Result, Step, quint_run, switch},
    std::{
        any::Any,
        collections::BTreeMap,
        marker::PhantomData,
        panic::{AssertUnwindSafe, catch_unwind},
        sync::Arc,
    },
};

type Node = EventHandlerTestContext<NullVoteHistoryStorage>;

/// A spec instance module: which ITF variable holds the environment, and the spec's processes.
trait Instance {
    /// Path to the environment variable `s` in each ITF state.
    const STATE_PATH: Path;
    /// `(process id, stake, is benevolent)` for every spec process, with stakes copied from the
    /// instance's `power` table. Byzantine processes get a stake but no node.
    const PROCESSES: &'static [(&'static str, u64, bool)];
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
}

/// Replays spec traces against one event handler per benevolent spec process.
struct VotorMbtDriver<I: Instance> {
    /// Kept alive for the banks and blockstore the nodes share.
    _fixtures: SharedFixtures,
    banks: Banks,
    /// Benevolent processes only, keyed by spec process id.
    nodes: BTreeMap<String, Node>,
    stats: ReplayStats,
    _instance: PhantomData<I>,
}

/// Counters reported when the driver is dropped, to show what the traces exercised.
#[derive(Default)]
struct ReplayStats {
    actions: BTreeMap<String, usize>,
    votes: usize,
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
            stats: ReplayStats::default(),
            _instance: PhantomData,
        }
    }

    /// Spec `init`: every benevolent process starts with `ParentReady(-1)` at slot 0.
    fn init(&mut self) -> Result {
        let ids: Vec<String> = self.nodes.keys().cloned().collect();
        for id in ids {
            self.node(&id)?.reset();
            self.parent_ready_event(&id, OFFSET, Block::default())?;
        }
        Ok(())
    }

    fn receive_block(&mut self, v: &str, block: SpecBlock) -> Result {
        let bank = self.banks.bank_for(&block);
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

    /// Passes `event` to the event handler of process `v`. Errors and panics are both turned
    /// into errors so that quint-connect reports the seed that reproduces them.
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
                BLSOp::PushVote { .. } => {
                    self.stats.votes = self.stats.votes.saturating_add(1);
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

impl<I: Instance> Driver for VotorMbtDriver<I> {
    // No state is compared yet.
    type State = ();

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
            init => self.init()?,
            // TODO(plafer): Remove since we upgraded to 0.33.0
            // Quint 0.32.0 sometimes labels the initial state of a trace with the step action's
            // name instead of `init`. `step` is never the innermost action of a later state,
            // since every branch of its `any` is a named action, so only the initial state can
            // carry it, and then without any nondet picks.
            step => {
                ensure!(
                    step.nondet_picks.get("v").is_none(),
                    "`step` reported as the action of a non-initial state"
                );
                self.init()?
            },
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
        let ReplayStats { actions, votes } = &self.stats;
        eprintln!("votor MBT replay: actions {actions:?}, votes pushed {votes}");
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
/// Run with: `cargo nextest run -p agave-votor --run-ignored ignored-only mbt`
#[ignore]
#[quint_run(
    spec = "src/event_handler/mbt/spec/statemachine.qnt",
    main = "some_byz",
    max_samples = 30
)]
fn mbt_votor_some_byz() -> impl Driver {
    VotorMbtDriver::<SomeByz>::new()
}
