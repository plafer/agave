//! Test fixture for driving [`EventHandler::handle_event`] synchronously.
//!
//! Shared by the event handler unit tests and the model-based tests. Fixtures that a whole
//! cluster shares (validator keypairs, bank forks, blockstore, leader schedule) are built once in
//! [`SharedFixtures`]. Each node then gets its own [`EventHandlerTestContext`] built from them by
//! [`setup_node`].
//!
//! [`EventHandler::handle_event`]: super::EventHandler

use {
    super::{LocalContext, PendingBlocks, stats::EventHandlerStats},
    crate::{
        commitment::CommitmentAggregationData,
        event::{LatestSwitchRequest, LeaderWindowInfo, RepairEventReceiver},
        root_utils::{self, RootContext},
        slot_clock::SharedAlpenglowSlotClock,
        timer_manager::TimerManager,
        vote_history::VoteHistory,
        vote_history_storage::{FileVoteHistoryStorage, VoteHistoryStorage},
        voting_service::BLSOp,
        voting_utils::VotingContext,
        votor::SharedContext,
    },
    agave_bls_sigverify::rewards::RewardInput,
    agave_votor_messages::{
        consensus_message::{BLS_KEYPAIR_DERIVE_SEED, Block, VoteMessage},
        metric_types::ConsensusMetricsEventReceiver,
        migration::MigrationStatus,
    },
    crossbeam_channel::{Receiver, Sender, bounded},
    parking_lot::RwLock as PlRwLock,
    solana_bls_signatures::keypair::Keypair as BLSKeypair,
    solana_clock::Slot,
    solana_gossip::{cluster_info::ClusterInfo, contact_info::ContactInfo},
    solana_keypair::Keypair,
    solana_ledger::{
        blockstore::Blockstore, blockstore_options::BlockstoreOptions, get_tmp_ledger_path,
        leader_schedule_cache::LeaderScheduleCache,
    },
    solana_net_utils::SocketAddrSpace,
    solana_pubkey::Pubkey,
    solana_runtime::{
        bank::{Bank, BankTestConfig},
        bank_forks::BankForks,
        bank_forks_controller::{BankForksController, BankForksControllerError},
        genesis_utils::{
            ValidatorVoteKeypairs, create_genesis_config_with_alpenglow_vote_accounts,
        },
        installed_scheduler_pool::BankWithScheduler,
    },
    solana_signer::Signer,
    solana_streamer::evicting_sender::EvictingSender,
    std::{
        collections::{BTreeSet, HashMap},
        sync::{
            Arc, RwLock,
            atomic::{AtomicBool, Ordering},
        },
    },
    tempfile::TempDir,
};

/// Fixtures shared by every node of a test cluster.
pub(super) struct SharedFixtures {
    pub(super) validator_keypairs: Vec<ValidatorVoteKeypairs>,
    /// Holds only the genesis bank (bank 0).
    pub(super) bank_forks: Arc<RwLock<BankForks>>,
    pub(super) blockstore: Arc<Blockstore>,
    pub(super) leader_schedule_cache: Arc<LeaderScheduleCache>,
    /// Bank hash details directory. Kept alive by every context built from these fixtures.
    pub(super) test_dir: Arc<TempDir>,
}

impl SharedFixtures {
    /// Creates one validator per entry in `stakes`, with that stake, and a genesis bank in which
    /// all of them have Alpenglow vote accounts.
    pub(super) fn new(stakes: Vec<u64>) -> Self {
        let validator_keypairs = (0..stakes.len())
            .map(|_| ValidatorVoteKeypairs::new(Keypair::new(), Keypair::new(), Keypair::new()))
            .collect::<Vec<_>>();
        let genesis = create_genesis_config_with_alpenglow_vote_accounts(
            1_000_000_000,
            &validator_keypairs,
            stakes,
        );
        let test_dir = TempDir::new().unwrap();
        let mut bank_test_config = BankTestConfig::default();
        bank_test_config.accounts_db_config.bank_hash_details_dir = test_dir.path().to_path_buf();
        let bank0 = Bank::new_with_paths_for_tests(
            &genesis.genesis_config,
            Some(bank_test_config),
            vec![],
            None,
        );
        let bank_forks = BankForks::new_rw_arc(bank0);
        let blockstore = Arc::new(
            Blockstore::open_with_options(
                &get_tmp_ledger_path!(),
                BlockstoreOptions::default_for_tests(),
            )
            .unwrap(),
        );
        let leader_schedule_cache = Arc::new(LeaderScheduleCache::new_from_bank(
            &bank_forks.read().unwrap().root_bank(),
        ));
        Self {
            validator_keypairs,
            bank_forks,
            blockstore,
            leader_schedule_cache,
            test_dir: Arc::new(test_dir),
        }
    }
}

pub(super) struct EventHandlerTestContext<S: VoteHistoryStorage = FileVoteHistoryStorage> {
    pub(super) bls_receiver: Receiver<BLSOp>,
    pub(super) commitment_receiver: Receiver<CommitmentAggregationData>,
    pub(super) own_vote_receiver: Receiver<VoteMessage>,
    // Keep receiver alive to prevent SenderDisconnected errors
    pub(super) own_reward_aggregates_receiver: Receiver<RewardInput>,
    pub(super) bank_forks: Arc<RwLock<BankForks>>,
    pub(super) my_bls_keypair: BLSKeypair,
    pub(super) timer_manager: Arc<PlRwLock<TimerManager>>,
    /// Exit flag of the thread spawned by `timer_manager`.
    pub(super) timer_exit: Arc<AtomicBool>,
    pub(super) leader_window_info_receiver: Receiver<LeaderWindowInfo>,
    pub(super) highest_parent_ready: Arc<RwLock<(Slot, Block)>>,
    pub(super) drop_bank_receiver: Receiver<Vec<BankWithScheduler>>,
    pub(super) cluster_info: Arc<ClusterInfo>,
    pub(super) consensus_metrics_receiver: ConsensusMetricsEventReceiver,
    // Keep receiver alive to prevent SenderDisconnected errors
    pub(super) repair_event_receiver: RepairEventReceiver,
    pub(super) shared_context: SharedContext,
    pub(super) voting_context: VotingContext,
    pub(super) root_context: RootContext,
    pub(super) local_context: LocalContext,
    pub(super) bls_ops: Vec<BLSOp>,
    pub(super) vote_history_storage: Arc<S>,
    // Keep the temp directory alive for vote history and bank hash details.
    _test_dir: Arc<TempDir>,
}

struct DirectBankForksController {
    my_pubkey: Pubkey,
    bank_forks: Arc<RwLock<BankForks>>,
    blockstore: Arc<Blockstore>,
    leader_schedule_cache: Arc<LeaderScheduleCache>,
    drop_bank_sender: Sender<Vec<BankWithScheduler>>,
}

impl BankForksController for DirectBankForksController {
    fn insert_bank(&self, bank: Bank) -> Result<BankWithScheduler, BankForksControllerError> {
        Ok(self.bank_forks.write().unwrap().insert(bank))
    }

    fn enqueue_set_root(&self, new_root: Block) {
        let new_root = new_root.slot;
        root_utils::check_and_handle_new_root(
            new_root,
            new_root,
            None,
            Some(new_root),
            &None,
            &self.drop_bank_sender,
            &self.blockstore,
            &self.leader_schedule_cache,
            &self.bank_forks,
            None,
            &self.my_pubkey,
            |_| {},
        );
    }

    fn clear_bank(&self, slot: Slot) -> Result<(), BankForksControllerError> {
        let bank_to_clear = self.bank_forks.read().unwrap().get_with_scheduler(slot);
        if let Some(bank) = bank_to_clear {
            let _ = bank.wait_for_completed_scheduler();
        }

        self.bank_forks.write().unwrap().clear_bank(slot, false);
        Ok(())
    }
}

/// Builds a context for validator 0 of a fresh 10-validator cluster, with vote history persisted
/// to a temporary directory.
pub(super) fn setup() -> EventHandlerTestContext {
    let stakes = (0..10_u64)
        .rev()
        .map(|i| 100_u64.saturating_add(i))
        .collect::<Vec<_>>();
    let fixtures = SharedFixtures::new(stakes);
    let vote_history_storage = Arc::new(FileVoteHistoryStorage::new(
        fixtures.test_dir.path().to_path_buf(),
    ));
    setup_node(&fixtures, 0, vote_history_storage)
}

/// Builds a context for the validator at `my_index` in `fixtures.validator_keypairs`.
pub(super) fn setup_node<S: VoteHistoryStorage + 'static>(
    fixtures: &SharedFixtures,
    my_index: usize,
    vote_history_storage: Arc<S>,
) -> EventHandlerTestContext<S> {
    let (bls_sender, bls_receiver) = bounded(1024);
    let (commitment_sender, commitment_receiver) = bounded(1024);
    let (own_vote_sender, own_vote_receiver) = EvictingSender::new_bounded(1024);
    let (reward_aggregates_sender, reward_aggregates_receiver) = bounded(1024);
    let (drop_bank_sender, drop_bank_receiver) = bounded(1024);
    let (consensus_metrics_sender, consensus_metrics_receiver) = bounded(1024);
    let (leader_window_info_sender, leader_window_info_receiver) = bounded(1024);
    let (repair_event_sender, repair_event_receiver) = bounded(1024);
    let latest_switch_request = LatestSwitchRequest::default();

    let my_keypairs = &fixtures.validator_keypairs[my_index];
    let my_node_keypair = my_keypairs.node_keypair.insecure_clone();
    let my_vote_keypair = my_keypairs.vote_keypair.insecure_clone();
    let my_bls_keypair =
        BLSKeypair::derive_from_signer(&my_vote_keypair, BLS_KEYPAIR_DERIVE_SEED).unwrap();
    let bank_forks = fixtures.bank_forks.clone();
    let contact_info = ContactInfo::new_localhost(&my_node_keypair.pubkey(), 0);
    let cluster_info = Arc::new(ClusterInfo::new(
        contact_info,
        Arc::new(my_node_keypair.insecure_clone()),
        SocketAddrSpace::Unspecified,
    ));
    let timer_exit = Arc::new(AtomicBool::new(false));
    let timer_manager = new_timer_manager(&cluster_info, timer_exit.clone());
    let blockstore = fixtures.blockstore.clone();
    let leader_schedule_cache = fixtures.leader_schedule_cache.clone();
    let bank_forks_controller = Arc::new(DirectBankForksController {
        my_pubkey: my_node_keypair.pubkey(),
        bank_forks: bank_forks.clone(),
        blockstore: blockstore.clone(),
        leader_schedule_cache: leader_schedule_cache.clone(),
        drop_bank_sender: drop_bank_sender.clone(),
    });
    let highest_parent_ready = Arc::new(RwLock::default());
    let alpenglow_slot_clock = SharedAlpenglowSlotClock::default();

    let shared_context = SharedContext {
        cluster_info: cluster_info.clone(),
        alpenglow_slot_clock,
        bank_forks: bank_forks.clone(),
        vote_history_storage: vote_history_storage.clone(),
        leader_window_info_sender,
        blockstore,
        highest_parent_ready: highest_parent_ready.clone(),
        repair_event_sender,
        latest_switch_request,
    };

    let voting_context = VotingContext {
        cluster_info: cluster_info.clone(),
        identity_keypair: Arc::new(my_node_keypair.insecure_clone()),
        sharable_banks: bank_forks.read().unwrap().sharable_banks(),
        vote_history: new_vote_history(my_node_keypair.pubkey()),
        bls_sender,
        commitment_sender,
        vote_account_pubkey: my_vote_keypair.pubkey(),
        wait_to_vote_slot: None,
        authorized_voter_keypairs: Arc::new(RwLock::new(vec![Arc::new(my_vote_keypair)])),
        vote_history_storage: vote_history_storage.clone(),
        derived_bls_keypairs: HashMap::new(),
        own_vote_sender,
        own_reward_sender: reward_aggregates_sender,
        consensus_metrics_sender,
        leader_schedule: leader_schedule_cache,
    };

    let root_context = RootContext {
        bank_notification_sender: None,
        bank_forks_controller,
    };

    EventHandlerTestContext {
        bls_receiver,
        commitment_receiver,
        own_vote_receiver,
        own_reward_aggregates_receiver: reward_aggregates_receiver,
        bank_forks,
        my_bls_keypair,
        timer_manager,
        timer_exit,
        leader_window_info_receiver,
        drop_bank_receiver,
        cluster_info,
        consensus_metrics_receiver,
        repair_event_receiver,
        highest_parent_ready,
        shared_context,
        voting_context,
        root_context,
        local_context: new_local_context(my_node_keypair.pubkey()),
        bls_ops: vec![],
        vote_history_storage,
        _test_dir: fixtures.test_dir.clone(),
    }
}

impl<S: VoteHistoryStorage> EventHandlerTestContext<S> {
    /// Returns this node to the state [`setup_node`] leaves it in, while keeping the shared
    /// fixtures, keypairs, and channels.
    ///
    /// Replaces the vote history, the local context, and the timer manager, and drains every
    /// receiver so that bounded channels never fill up across many resets.
    pub(super) fn reset(&mut self) {
        // The old timer thread never gets past `wait_for_migration_or_exit` (migration is never
        // completed in these tests), which only re-checks the exit flag every 5s. Joining it
        // would stall every reset, so signal it and let it exit on its own.
        self.timer_exit.store(true, Ordering::Relaxed);
        self.timer_exit = Arc::new(AtomicBool::new(false));
        self.timer_manager = new_timer_manager(&self.cluster_info, self.timer_exit.clone());

        let my_pubkey = self.cluster_info.id();
        self.voting_context.vote_history = new_vote_history(my_pubkey);
        self.local_context = new_local_context(my_pubkey);
        self.shared_context.alpenglow_slot_clock = SharedAlpenglowSlotClock::default();
        self.shared_context.latest_switch_request = LatestSwitchRequest::default();
        *self.highest_parent_ready.write().unwrap() = <(Slot, Block)>::default();
        self.bls_ops.clear();

        drain(&self.bls_receiver);
        drain(&self.commitment_receiver);
        drain(&self.own_vote_receiver);
        drain(&self.own_reward_aggregates_receiver);
        drain(&self.leader_window_info_receiver);
        drain(&self.drop_bank_receiver);
        drain(&self.consensus_metrics_receiver);
        drain(&self.repair_event_receiver);
    }
}

fn new_timer_manager(
    cluster_info: &Arc<ClusterInfo>,
    exit: Arc<AtomicBool>,
) -> Arc<PlRwLock<TimerManager>> {
    // The receiver is dropped right away: tests inject timeouts themselves and only inspect
    // which timers are set.
    let (event_sender, _event_receiver) = bounded(1024);
    Arc::new(PlRwLock::new(TimerManager::new(
        cluster_info.clone(),
        event_sender,
        exit,
        Arc::default(),
        Arc::new(MigrationStatus::default()),
    )))
}

fn new_vote_history(my_pubkey: Pubkey) -> VoteHistory {
    let mut vote_history = VoteHistory::new(my_pubkey, 0);
    vote_history.initialize_genesis(Block::default());
    vote_history
}

fn new_local_context(my_pubkey: Pubkey) -> LocalContext {
    LocalContext {
        my_pubkey,
        genesis_block: Block::default(),
        pending_blocks: PendingBlocks::new(),
        finalized_blocks: BTreeSet::new(),
        received_shred: BTreeSet::new(),
        stats: EventHandlerStats::default(),
        standstill_slot: None,
    }
}

fn drain<T>(receiver: &Receiver<T>) {
    while receiver.try_recv().is_ok() {}
}
