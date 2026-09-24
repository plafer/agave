//! Maps the spec's slots, hashes, and blocks onto agave values.
//!
//! - Spec slot `s` is agave slot `s + OFFSET`. With `OFFSET = 4`, spec slot 0 is the first slot
//!   of agave's second leader window, so agave's `slot == 1` special case for the first window is
//!   never reached, and every spec window maps onto exactly one agave window.
//! - Spec hash `-1` (the parent of the first block) is `Hash::default()`, the parent block id
//!   agave reports for children of the genesis bank. Every other spec hash `h` maps to a fixed,
//!   distinct `Hash`.
//! - Spec blocks are turned into frozen banks that are never inserted into `BankForks`.

use {
    agave_votor_messages::consensus_message::Block,
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

/// Maps a spec hash onto an agave block id.
pub(super) fn hash_of(h: i64) -> Hash {
    if h == GENESIS_HASH {
        return Hash::default();
    }
    let mut bytes = [0_u8; 32];
    bytes[..8].copy_from_slice(&h.to_le_bytes());
    // Keeps every mapped hash distinct from `Hash::default()`, including for `h == 0`.
    bytes[31] = 0xa1;
    Hash::new_from_array(bytes)
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

/// Frozen banks for spec blocks, built lazily and cached across traces.
pub(super) struct Banks {
    genesis: Arc<Bank>,
    by_hash: HashMap<i64, Arc<Bank>>,
}

impl Banks {
    pub(super) fn new(genesis: Arc<Bank>) -> Self {
        Self {
            genesis,
            by_hash: HashMap::new(),
        }
    }

    /// Returns the frozen bank for `block`, with `slot = agave_slot(block.slot)` and
    /// `block_id = hash_of(block.hash)`.
    ///
    /// Its parent is, in order of preference:
    /// - the genesis bank, if `block.parent` is the genesis hash;
    /// - the bank already built for `block.parent`;
    /// - a phantom frozen bank at the slot just before `block` with `block_id =
    ///   hash_of(block.parent)`. Placing it at `slot - 1` makes agave's "consecutive parent" check
    ///   pass, so the decision falls through to whether the node voted to notarize the parent,
    ///   mirroring the spec's `VotedNotar(b.parent) ∈ state[b.slot - 1]`.
    pub(super) fn bank_for(&mut self, block: &SpecBlock) -> Arc<Bank> {
        let slot = agave_slot(block.slot);
        if let Some(bank) = self.by_hash.get(&block.hash) {
            assert_eq!(
                (bank.slot(), bank.block_id()),
                (slot, Some(hash_of(block.hash))),
                "spec hash {} reused for a different block",
                block.hash
            );
            return bank.clone();
        }
        let parent = if block.parent == GENESIS_HASH {
            self.genesis.clone()
        } else if let Some(parent) = self.by_hash.get(&block.parent) {
            parent.clone()
        } else {
            let phantom_slot = slot
                .checked_sub(1)
                .expect("agave slots of spec blocks are positive");
            frozen_bank(self.genesis.clone(), phantom_slot, hash_of(block.parent))
        };
        let bank = frozen_bank(parent, slot, hash_of(block.hash));
        self.by_hash.insert(block.hash, bank.clone());
        bank
    }
}

fn frozen_bank(parent: Arc<Bank>, slot: Slot, block_id: Hash) -> Arc<Bank> {
    let bank = Bank::new_from_parent(parent, SlotLeader::new_unique(), slot);
    bank.set_block_id(Some(block_id));
    bank.freeze();
    Arc::new(bank)
}
