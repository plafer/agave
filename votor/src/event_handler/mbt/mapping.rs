//! Maps the spec's slots, hashes, and blocks onto agave values.
//!
//! - Spec slot `s` is agave slot `s + OFFSET`. With `OFFSET = 4`, spec slot 0 is the first slot
//!   of agave's second leader window, so agave's `slot == 1` special case for the first window is
//!   never reached, and every spec window maps onto exactly one agave window.
//! - Spec hash `-1` (the parent of the first block) is `Hash::default()`, the parent block id
//!   agave reports for children of the genesis bank. Every other spec hash `h` maps to a fixed,
//!   distinct `Hash`.
//! - Spec blocks are turned into frozen banks that are never inserted into `BankForks`.
//! - Votes map back onto spec messages through the inverses of the slot and hash mappings.

use {
    super::spec_types::{BlockRef, Message},
    agave_votor_messages::{consensus_message::Block, vote::Vote},
    anyhow::{Context, Result, anyhow, bail, ensure},
    serde::Deserialize,
    solana_clock::Slot,
    solana_hash::Hash,
    solana_runtime::bank::{Bank, SlotLeader},
    std::{collections::HashMap, sync::Arc},
};

/// Agave slot of spec slot 0.
pub(super) const OFFSET: Slot = 4;

/// Spec hash of the (implicit) parent of the first block.
const GENESIS_HASH: i64 = -1;

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

impl SpecBlock {
    pub(super) const fn new(slot: i64, hash: i64, parent: i64) -> Self {
        Self { slot, hash, parent }
    }
}

/// Frozen banks for spec blocks, built lazily and cached across traces.
pub(super) struct Banks {
    genesis: Arc<Bank>,
    /// Every block of the spec instance (`allBlocks`).
    blocks: &'static [SpecBlock],
    by_hash: HashMap<i64, Arc<Bank>>,
}

impl Banks {
    pub(super) fn new(genesis: Arc<Bank>, blocks: &'static [SpecBlock]) -> Self {
        Self {
            genesis,
            blocks,
            by_hash: HashMap::new(),
        }
    }

    /// Returns the frozen bank for `block`, with `slot = agave_slot(block.slot)` and
    /// `block_id = hash_of(block.hash)`.
    ///
    /// Its parent is, in order of preference:
    /// - the genesis bank, if `block.parent` is the genesis hash;
    /// - the bank for `block.parent`, if that is a block of the instance (built first if needed);
    /// - a phantom frozen bank at the slot just before `block` with `block_id =
    ///   hash_of(block.parent)`. Placing it at `slot - 1` makes agave's "consecutive parent" check
    ///   pass, so the decision falls through to whether the node voted to notarize the parent,
    ///   mirroring the spec's `VotedNotar(b.parent) ∈ state[b.slot - 1]`. The spec never
    ///   notarizes a hash that is not a block, so neither side ever votes for such a child.
    pub(super) fn bank_for(&mut self, block: &SpecBlock) -> Result<Arc<Bank>> {
        let blocks = self.blocks;
        ensure!(
            blocks.contains(block),
            "{block:?} is not in the driver's copy of the instance's blocks"
        );
        let slot = agave_slot(block.slot);
        if let Some(bank) = self.by_hash.get(&block.hash) {
            ensure!(
                (bank.slot(), bank.block_id()) == (slot, Some(hash_of(block.hash))),
                "spec hash {} reused for a different block",
                block.hash
            );
            return Ok(bank.clone());
        }
        let parent = if block.parent == GENESIS_HASH {
            self.genesis.clone()
        } else if let Some(&parent) = blocks.iter().find(|b| b.hash == block.parent) {
            ensure!(
                parent.slot < block.slot,
                "parent {parent:?} of {block:?} is not in an earlier slot"
            );
            self.bank_for(&parent)?
        } else {
            let phantom_slot = slot
                .checked_sub(1)
                .expect("agave slots of spec blocks are positive");
            frozen_bank(self.genesis.clone(), phantom_slot, hash_of(block.parent))
        };
        let bank = frozen_bank(parent, slot, hash_of(block.hash));
        self.by_hash.insert(block.hash, bank.clone());
        Ok(bank)
    }
}

fn frozen_bank(parent: Arc<Bank>, slot: Slot, block_id: Hash) -> Arc<Bank> {
    let bank = Bank::new_from_parent(parent, SlotLeader::new_unique(), slot);
    bank.set_block_id(Some(block_id));
    bank.freeze();
    Arc::new(bank)
}
