//! One agave [`ConsensusPool`] per benevolent spec process, driven synchronously.
//!
//! The spec's environment delivers every vote to every process the moment it is broadcast, and
//! computes the pool conditions over all messages. The driver mirrors this: every vote a node
//! pushes reaches its own pool as [`PoolVote::Own`] and every other pool as
//! [`PoolVote::External`] before the next step. The pool decides which derived events exist, and
//! the spec decides when each one is delivered: a spec action for a derived event must find the
//! event among the ones the node's pool emitted, and the driver passes the pool's event to the
//! node's event handler once.
//!
//! Pools only run in instances without Byzantine processes. The spec's Byzantine soup lets a
//! Byzantine process send every possible vote, including conflicting ones, and the spec's
//! conditions count each sender once. Agave's pool adds up stake per vote, so a rank that sends,
//! say, both a notarize and a skip vote in a slot counts twice. In production, bls-sigverify's
//! conflict filter stops such votes before they reach the pool, and that filter is private to
//! `agave-bls-sigverify`. Correct processes never send conflicting votes, so for them the sums
//! equal the spec's unions.

use {
    super::{
        mapping::{GENESIS_HASH, OFFSET, spec_hash, spec_slot, to_spec_certificate},
        spec_types::{Certificate, PoolView},
    },
    crate::{
        consensus_pool::ConsensusPool,
        consensus_pool_service::{PoolMessage, PoolVote},
        event::VotorEvent,
    },
    agave_bls_sigverify::generated_cert_types::GeneratedCertTypes,
    agave_votor_messages::{
        certificate::CertificateType, consensus_message::Block, migration::MigrationStatus,
    },
    anyhow::{Result, bail},
    solana_clock::Slot,
    solana_gossip::cluster_info::ClusterInfo,
    solana_runtime::bank::Bank,
    std::{collections::BTreeSet, sync::Arc},
};

/// A derived event a pool emitted, in spec-independent agave terms.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum PoolEvent {
    BlockNotarized(Block),
    ParentReady { slot: Slot, parent: Block },
    SafeToNotar(Block),
    SafeToSkip(Slot),
    BlockNotarFallback(Block),
    Finalized(Block, bool),
}

impl PoolEvent {
    /// The pool's counterpart of `event`. Fails for events a pool never emits.
    fn from_votor_event(event: VotorEvent) -> Result<Self> {
        Ok(match event {
            VotorEvent::BlockNotarized(block) => PoolEvent::BlockNotarized(block),
            VotorEvent::ParentReady { slot, parent_block } => PoolEvent::ParentReady {
                slot,
                parent: parent_block,
            },
            VotorEvent::SafeToNotar(block) => PoolEvent::SafeToNotar(block),
            VotorEvent::SafeToSkip(slot) => PoolEvent::SafeToSkip(slot),
            VotorEvent::BlockNotarFallback(block) => PoolEvent::BlockNotarFallback(block),
            VotorEvent::Finalized(block, fast) => PoolEvent::Finalized(block, fast),
            VotorEvent::Block(_)
            | VotorEvent::FirstShred(_)
            | VotorEvent::TimeoutCrashedLeader(_)
            | VotorEvent::Timeout(_)
            | VotorEvent::ProduceWindow(_)
            | VotorEvent::Standstill(_)
            | VotorEvent::SetIdentity => bail!("a consensus pool emitted {event:?}"),
        })
    }

    fn to_votor_event(self) -> VotorEvent {
        match self {
            PoolEvent::BlockNotarized(block) => VotorEvent::BlockNotarized(block),
            PoolEvent::ParentReady { slot, parent } => VotorEvent::ParentReady {
                slot,
                parent_block: parent,
            },
            PoolEvent::SafeToNotar(block) => VotorEvent::SafeToNotar(block),
            PoolEvent::SafeToSkip(slot) => VotorEvent::SafeToSkip(slot),
            PoolEvent::BlockNotarFallback(block) => VotorEvent::BlockNotarFallback(block),
            PoolEvent::Finalized(block, fast) => VotorEvent::Finalized(block, fast),
        }
    }

    /// The name of the event's kind, for statistics.
    pub(super) fn kind(&self) -> &'static str {
        match self {
            PoolEvent::BlockNotarized(_) => "BlockNotarized",
            PoolEvent::ParentReady { .. } => "ParentReady",
            PoolEvent::SafeToNotar(_) => "SafeToNotar",
            PoolEvent::SafeToSkip(_) => "SafeToSkip",
            PoolEvent::BlockNotarFallback(_) => "BlockNotarFallback",
            PoolEvent::Finalized(_, false) => "Finalized(slow)",
            PoolEvent::Finalized(_, true) => "Finalized(fast)",
        }
    }
}

/// The consensus pool of one node, with the derived events it emitted and the ones the driver
/// passed on to the node's event handler.
pub(super) struct NodePool {
    pool: ConsensusPool,
    /// Every event the pool emitted since `init`.
    emitted: BTreeSet<PoolEvent>,
    /// The emitted events that were passed to the node's event handler.
    dispatched: BTreeSet<PoolEvent>,
}

impl NodePool {
    /// Creates the pool of the node with `cluster_info`, rooted at `root_bank`. The pool starts
    /// with slot `OFFSET` parent-ready for the genesis block, as the event handler does: this is
    /// the initial parent ready of the consensus pool service, which the driver sends to the event
    /// handler itself.
    pub(super) fn new(cluster_info: Arc<ClusterInfo>, root_bank: &Bank) -> Self {
        let pool = ConsensusPool::new(
            cluster_info,
            root_bank,
            Arc::new(GeneratedCertTypes::default()),
            Arc::new(MigrationStatus::post_migration_status()),
            (OFFSET, Block::default()),
        );
        Self {
            pool,
            emitted: BTreeSet::new(),
            dispatched: BTreeSet::new(),
        }
    }

    /// Adds `votes` to the pool, as the consensus pool service does, and records the events the
    /// pool emits. Returns the events that the pool had not emitted before.
    pub(super) fn add_votes(
        &mut self,
        root_bank: &Bank,
        votes: Vec<PoolVote>,
    ) -> Result<Vec<PoolEvent>> {
        self.pool.maybe_prune(root_bank.slot());
        let mut events = Vec::new();
        self.pool
            .add_pool_msg(root_bank, PoolMessage::Votes(votes), &mut events);
        let mut new_events = Vec::new();
        for event in events {
            let event = PoolEvent::from_votor_event(event)?;
            if self.emitted.insert(event) {
                new_events.push(event);
            }
        }
        Ok(new_events)
    }

    /// Returns `event` for the node's event handler the first time it is asked for, and `None`
    /// after that, since agave delivers each derived event once. Fails unless the pool emitted
    /// `event`, with an error that completes "the pool ...".
    pub(super) fn take_for_dispatch(&mut self, event: &PoolEvent) -> Result<Option<VotorEvent>> {
        if !self.emitted.contains(event) {
            let same_kind: Vec<&PoolEvent> = self
                .emitted
                .iter()
                .filter(|emitted| emitted.kind() == event.kind())
                .collect();
            let certificates: Vec<&CertificateType> = self.certificate_types().collect();
            bail!(
                "has no {event:?}; its {} events are {same_kind:?}, and it holds the certificates \
                 {certificates:?}",
                event.kind()
            );
        }
        Ok(self
            .dispatched
            .insert(*event)
            .then(|| event.to_votor_event()))
    }

    /// The types of the certificates the pool holds.
    pub(super) fn certificate_types(&self) -> impl Iterator<Item = &CertificateType> {
        self.pool.completed_certificate_types()
    }

    /// Projects the pool onto the spec's `PoolView` for spec slots `0..num_slots`: the
    /// certificates it holds, and the parent-ready pairs it emitted together with the initial
    /// one. Certificates and pairs past the last spec slot are left out, since the spec has no
    /// state there.
    ///
    /// The spec's certificates are conditions on the votes, and a notarization certificate's
    /// votes also make a notar-fallback certificate. Agave builds at most one of the
    /// certificates a block's notarize votes make, the strongest, and treats `Notarize` and
    /// `FinalizeFast` as notar-fallback-or-stronger (as `ParentReadyTracker` does). So each of
    /// them also projects onto the block's `NotarFallbackCertificate`. `FinalizeFast` does not
    /// project onto `NotarizationCertificate`: agave does not treat it as one, and emits no
    /// `BlockNotarized` for it.
    pub(super) fn view(&self, num_slots: usize) -> Result<PoolView> {
        let in_spec = |slot: Slot| {
            spec_slot(slot)
                .ok()
                .and_then(|s| usize::try_from(s).ok())
                .is_some_and(|s| s < num_slots)
        };
        let mut certificates = BTreeSet::new();
        for certificate in self.certificate_types() {
            if !in_spec(certificate.slot()) {
                continue;
            }
            let spec_certificate = to_spec_certificate(certificate)?;
            certificates.insert(spec_certificate);
            match spec_certificate {
                Certificate::NotarizationCertificate(block)
                | Certificate::FastFinalizationCertificate(block) => {
                    certificates.insert(Certificate::NotarFallbackCertificate(block));
                }
                Certificate::NotarFallbackCertificate(_)
                | Certificate::SkipCertificate(_)
                | Certificate::FinalizationCertificate(_) => {}
            }
        }
        let mut parent_ready = BTreeSet::from([(spec_slot(OFFSET)?, GENESIS_HASH)]);
        for event in &self.emitted {
            if let PoolEvent::ParentReady { slot, parent } = event
                && in_spec(*slot)
            {
                parent_ready.insert((spec_slot(*slot)?, spec_hash(&parent.block_id)?));
            }
        }
        Ok(PoolView {
            certificates,
            parent_ready,
        })
    }
}
