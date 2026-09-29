//! Maps the spec's slots, hashes, and blocks onto agave values.
//!
//! - Spec slot `s` is agave slot `s + OFFSET`. With `OFFSET = 4`, spec slot 0 is the first slot
//!   of agave's second leader window, so agave's `slot == 1` special case for the first window is
//!   never reached, and every spec window maps onto exactly one agave window.
//! - Spec hash `-1` (the parent of the first block) is `Hash::default()`, the parent block id
//!   agave reports for children of the genesis bank. Every other spec hash `h` maps to a fixed,
//!   distinct `Hash`.
//! - Spec blocks are turned into frozen banks that are never inserted into `BankForks`.
//! - Votes and agave's pending blocks map back onto spec values through the inverses of the slot
//!   and hash mappings.

use {
    super::spec_types::{BlockRef, Message},
    agave_votor_messages::{consensus_message::Block, vote::Vote},
    anyhow::{Context, Result, anyhow, bail, ensure},
    serde::Deserialize,
    solana_clock::Slot,
    solana_hash::Hash,
    solana_runtime::bank::{Bank, SlotLeader},
    std::{
        collections::{BTreeSet, HashMap},
        sync::Arc,
    },
};

/// Agave slot of spec slot 0.
pub(super) const OFFSET: Slot = 4;

/// Spec hash of the (implicit) parent of the first block.
pub(super) const GENESIS_HASH: i64 = -1;

/// Maps a spec slot onto an agave slot.
pub(super) fn agave_slot(s: i64) -> Slot {
    let s = Slot::try_from(s).unwrap_or_else(|_| panic!("spec slot {s} must be non-negative"));
    s.checked_add(OFFSET).expect("spec slot overflow")
}

/// Inverse of [`agave_slot`].
pub(super) fn spec_slot(slot: Slot) -> Result<i64> {
    let s = slot
        .checked_sub(OFFSET)
        .ok_or_else(|| anyhow!("agave slot {slot} is below spec slot 0"))?;
    i64::try_from(s).context("agave slot out of range")
}

/// Last byte of every block id that [`hash_of`] mints. Keeps every mapped hash distinct from
/// `Hash::default()`, including for `h == 0`.
const HASH_MARKER: u8 = 0xa1;

/// Maps a spec hash onto an agave block id.
pub(super) fn hash_of(h: i64) -> Hash {
    if h == GENESIS_HASH {
        return Hash::default();
    }
    let mut bytes = [0_u8; 32];
    bytes[..8].copy_from_slice(&h.to_le_bytes());
    bytes[31] = HASH_MARKER;
    Hash::new_from_array(bytes)
}

/// Inverse of [`hash_of`].
pub(super) fn spec_hash(hash: &Hash) -> Result<i64> {
    if *hash == Hash::default() {
        return Ok(GENESIS_HASH);
    }
    let bytes = hash.as_bytes();
    let (h, rest) = bytes.split_at(8);
    ensure!(
        rest.iter().rev().skip(1).all(|&b| b == 0) && rest.last() == Some(&HASH_MARKER),
        "block id {hash} was not minted by `hash_of`"
    );
    Ok(i64::from_le_bytes(h.try_into().expect("8 bytes")))
}

/// Maps an agave block back onto a spec block reference.
fn spec_block_ref(block: &Block) -> Result<BlockRef> {
    Ok(BlockRef {
        slot: spec_slot(block.slot)?,
        hash: spec_hash(&block.block_id)?,
    })
}

/// Maps a vote cast by agave onto the spec message it broadcasts.
pub(super) fn to_spec_message(vote: &Vote) -> Result<Message> {
    Ok(match vote {
        Vote::Notarize(vote) => Message::NotarVoteMsg(spec_block_ref(&vote.block)?),
        Vote::NotarizeFallback(vote) => Message::NotarFallBackVoteMsg(spec_block_ref(&vote.block)?),
        Vote::Skip(vote) => Message::SkipVoteMsg(spec_slot(vote.slot)?),
        Vote::SkipFallback(vote) => Message::SkipFallbackVoteMsg(spec_slot(vote.slot)?),
        Vote::Finalize(vote) => Message::FinalVoteMsg(spec_slot(vote.slot)?),
        Vote::Genesis(_) => bail!("genesis votes have no spec counterpart: {vote:?}"),
    })
}

/// Maps a block that agave keeps pending, together with the parent agave read from its bank,
/// back onto the spec block it was built from.
pub(super) fn to_spec_block(block: &Block, parent: &Block) -> Result<SpecBlock> {
    Ok(SpecBlock {
        slot: spec_slot(block.slot)?,
        hash: spec_hash(&block.block_id)?,
        parent: spec_hash(&parent.block_id)?,
    })
}

/// Maps a spec block reference onto an agave block. The genesis hash maps to the genesis block
/// regardless of `slot`.
pub(super) fn block_ref(slot: i64, hash: i64) -> Block {
    if hash == GENESIS_HASH {
        return Block::default();
    }
    Block {
        slot: agave_slot(slot),
        block_id: hash_of(hash).into(),
    }
}

/// Mirror of the spec's `Block`.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(super) struct SpecBlock {
    pub(super) slot: i64,
    pub(super) hash: i64,
    pub(super) parent: i64,
}

/// Frozen banks for the blocks of the current trace, built at `init` from the trace's block set.
pub(super) struct Banks {
    genesis: Arc<Bank>,
    /// The trace's blocks (the spec environment's `blocks`).
    blocks: BTreeSet<SpecBlock>,
    /// The bank of every block in `blocks`, keyed by its spec hash.
    by_hash: HashMap<i64, Arc<Bank>>,
}

impl Banks {
    /// Creates an empty set of banks. [`Banks::reset`] must be called before any lookup.
    pub(super) fn new(genesis: Arc<Bank>) -> Self {
        Self {
            genesis,
            blocks: BTreeSet::new(),
            by_hash: HashMap::new(),
        }
    }

    /// Makes `blocks` the trace's block set, and builds a frozen bank for each of them, with
    /// `slot = agave_slot(block.slot)` and `block_id = hash_of(block.hash)`. Keeps the existing
    /// banks if `blocks` equals the previous trace's set.
    ///
    /// Blocks are built in slot order. A block's parent bank is:
    /// - the genesis bank, if `block.parent` is the genesis hash;
    /// - the bank for `block.parent`, if that is one of `blocks`;
    /// - otherwise a phantom frozen bank at the slot just before `block` with `block_id =
    ///   hash_of(block.parent)`. Placing it at `slot - 1` makes agave's "consecutive parent" check
    ///   pass, so the decision falls through to whether the node voted to notarize the parent,
    ///   mirroring the spec's `VotedNotar(b.parent) ∈ state[b.slot - 1]`. The spec never
    ///   notarizes a hash that is not a block, so neither side ever votes for such a child.
    ///
    /// Fails if two blocks share a hash, or if a block's parent is one of `blocks` but not in an
    /// earlier slot.
    pub(super) fn reset(&mut self, blocks: BTreeSet<SpecBlock>) -> Result<()> {
        if blocks == self.blocks {
            return Ok(());
        }
        self.blocks.clear();
        self.by_hash.clear();

        let mut by_hash: HashMap<i64, SpecBlock> = HashMap::with_capacity(blocks.len());
        for &block in &blocks {
            ensure!(
                block.hash != GENESIS_HASH,
                "{block:?} uses the genesis hash {GENESIS_HASH}"
            );
            if let Some(other) = by_hash.insert(block.hash, block) {
                bail!("{other:?} and {block:?} share a hash");
            }
        }

        // `SpecBlock` orders by slot first, so every parent in `blocks` is built before its
        // children.
        let mut banks: HashMap<i64, Arc<Bank>> = HashMap::with_capacity(blocks.len());
        for block in &blocks {
            let slot = agave_slot(block.slot);
            let parent = if block.parent == GENESIS_HASH {
                self.genesis.clone()
            } else if let Some(parent) = by_hash.get(&block.parent) {
                ensure!(
                    parent.slot < block.slot,
                    "parent {parent:?} of {block:?} is not in an earlier slot"
                );
                banks
                    .get(&parent.hash)
                    .expect("blocks of earlier slots are built first")
                    .clone()
            } else {
                let phantom_slot = slot
                    .checked_sub(1)
                    .expect("agave slots of spec blocks are positive");
                frozen_bank(self.genesis.clone(), phantom_slot, hash_of(block.parent))
            };
            banks.insert(block.hash, frozen_bank(parent, slot, hash_of(block.hash)));
        }

        self.blocks = blocks;
        self.by_hash = banks;
        Ok(())
    }

    /// Returns the frozen bank built for `block` by [`Banks::reset`]. Fails if `block` is not one
    /// of the trace's blocks.
    pub(super) fn bank_for(&self, block: &SpecBlock) -> Result<Arc<Bank>> {
        ensure!(
            self.blocks.contains(block),
            "{block:?} is not one of the trace's blocks"
        );
        Ok(self
            .by_hash
            .get(&block.hash)
            .expect("every block of the trace has a bank")
            .clone())
    }

    /// The trace's blocks.
    pub(super) fn blocks(&self) -> &BTreeSet<SpecBlock> {
        &self.blocks
    }
}

fn frozen_bank(parent: Arc<Bank>, slot: Slot, block_id: Hash) -> Arc<Bank> {
    let bank = Bank::new_from_parent(parent, SlotLeader::new_unique(), slot);
    bank.set_block_id(Some(block_id));
    bank.freeze();
    Arc::new(bank)
}
