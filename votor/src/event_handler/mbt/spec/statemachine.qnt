module consensus {
  import basicSpells.* from "./basicSpells"
  import alpenglow.* from "./alpenglow"
  export alpenglow.*

  /// agave patch 13: what a pool that holds every message knows: the certificates that
  /// `isCertified` holds for, and the pairs `(s, hash(b))` for which `ParentReady(s, hash(b))`
  /// is enabled. Also, per benevolent process, the blocks and slots for which `SafeToNotar` and
  /// `SafeToSkip` are enabled; unlike the rest, these depend on the process's own votes.
  type PoolView = {
    certificates: Set[Certificate],
    parentReady: Set[(Slot, Blockhash)],
    safeToNotar: ProcessID -> Set[BlockReference],
    safeToSkip: ProcessID -> Set[Slot],
  }

  /// This is all the information of the global state
  type Environment = {
    // agave patch 7: the blocks of this trace, fixed by `init` (upstream: the constant
    // `allBlocks`). Every action and invariant reads the blocks from here.
    blocks: Set[Block],

    // The local state of each process
    system: ProcessID -> LocalState,

    // Messages sent so far, a.k.a. the message soup
    msgBuffer: Set[NetworkMsg],

    // Started timeouts per process
    activeTimeouts: ProcessID -> Set[TimeoutEventData],

    // agave patch 13: an observation of the pool over all messages, recomputed after every
    // step (see `withPoolView`). No action reads it; the model-based tests compare it with
    // agave's consensus pools.
    pool: PoolView,
  }

  /// Interface consensus algorithm / state machine: how consensus algorithm effects the environment
  pure def applyEffect(env: Environment, v: ProcessID, res: Result): Environment =
    // update the state of the process
    val new = { ... env, system: env.system.set(v, res.post) } 
    // update the message buffer and active timeouts
    res.output.fold(new, (s, o) =>
      match o {
        | Broadcast(msg) =>
          val nm = { sender: v, msg: msg }
          { ...s, msgBuffer: s.msgBuffer.union(Set(nm)) }
        | ScheduleEventTimeout(slot) =>
          { ...s, activeTimeouts: s.activeTimeouts.set(v, s.activeTimeouts.get(v).union(Set(slot))) }
      })


  //
  //
  // State Machine
  //
  //

  var s: Environment

  // agave patch 7: over `s.blocks` (upstream: `allBlocks`, via `finalized`).
  val finalizedBlocks: Slot -> Set[Blockhash] =
    aliveSlots.mapBy(slot => finalized(slot, allMessages, s.blocks))

  var ch: Slot -> Set[Blockhash] // blockchain as seen by message soup. Ancestors not seen as finalized here if they are not finalized themselves


  var counter: int // Required for TLC

  /// all the messages that could be received, i.e., the ones sent by benevolent processes
  /// and the ones that byzantine processes may send. We use this in the actions to allow
  /// processes to respond to byzantine behavior. At the same time, we don't clutter
  /// msgBuffer with byzantine messages.
  /// agave patch 10: over `byzSoup` (upstream: `byzNetworkMsgs`).
  val allMessages = s.msgBuffer.union(byzSoup)

  /// agave patch 10: whether `hash` is a `genHash` (agave patch 8) of a slot other than `slot`.
  /// The fixed instances' hashes are below 100, so this never holds for them.
  pure def isGenHashOfOtherSlot(slot: Slot, hash: Blockhash): bool =
    hash >= 100 and (hash - 100) / 10 != slot

  /// agave patch 10: the Byzantine messages, without the votes for a block reference whose hash
  /// is a `genHash` of another slot. No generated block has such a reference, and every slot
  /// keeps the votes for its own `genHash`es, so no certificate or condition on the trace's
  /// blocks changes. In `agave_gen` this leaves 24 of the 192 block references and makes
  /// `quint run` about three times faster. For the fixed instances it is `byzNetworkMsgs`.
  pure val byzSoup = byzNetworkMsgs.filter(m => match m.msg {
    | NotarVoteMsg(r) => not(isGenHashOfOtherSlot(r.slot, r.hash))
    | NotarFallBackVoteMsg(r) => not(isGenHashOfOtherSlot(r.slot, r.hash))
    | _ => true
  })

  /// agave patch 3: number of slots each process keeps state for. Covers `windowSlots(slot)`
  /// for every alive slot, since `trySkipWindow` touches the whole window (upstream: a fixed 5).
  pure val numSlots = 4 * (aliveSlots.fold(0, (m, x) => max(m, x)) / 4 + 1)

  /// agave patch 13: the pool view over the messages `msgs` and the trace's `blocks`.
  /// `parentReady` always has `(0, -1)`, the parent ready that `init` preloads.
  pure def poolView(msgs: Set[NetworkMsg], blocks: Set[Block]): PoolView = {
    // The votes of a certificate are all in its slot, so each candidate is only checked against
    // the messages of its slot. That gives the same result, and makes `quint run` faster.
    val msgsBySlot = aliveSlots.union(blocks.map(b => b.slot))
      .mapBy(slot => msgs.filter(m => slotOf(m.msg) == slot))
    val blockCertificates = blocks.map(b => b.reference()).map(r => Set(
      FastFinalizationCertificate(r),
      NotarizationCertificate(r),
      NotarFallbackCertificate(r),
    ).filter(c => isCertified(c, msgsBySlot.get(r.slot)))).flatten()
    val parentCertified = blocks.mapBy(b => parentNotarFallbackCertified(b, msgs, blocks))
    val slotCertificates = aliveSlots.map(slot => Set(
      SkipCertificate(slot),
      FinalizationCertificate(slot),
    ).filter(c => isCertified(c, msgsBySlot.get(slot)))).flatten()
    {
      certificates: blockCertificates.union(slotCertificates),
      parentReady: Set((0, -1)).union(aliveSlots.map(slot =>
        parentReadyBlocks(slot, msgs, blocks).map(b => (slot, b.hash))
      ).flatten()),
      safeToNotar: benevolent.mapBy(v =>
        blocks.filter(b =>
          safeToNotarHolds(v, b, msgsBySlot.get(b.slot), parentCertified.get(b))
        ).map(b => b.reference())
      ),
      safeToSkip: benevolent.mapBy(v =>
        aliveSlots.filter(slot => safeToSkipHolds(v, slot, msgsBySlot.get(slot)))
      ),
    }
  }

  /// agave patch 13: `env` with its pool view brought up to date. Only in instances without
  /// Byzantine processes, the ones where the model-based tests run agave's consensus pools
  /// (patch 12); elsewhere the view stays empty. Computing it over the Byzantine soup of
  /// `agave_gen` more than doubled the time `quint run` takes there.
  pure def withPoolView(env: Environment): Environment =
    if (byzantine == Set()) { ...env, pool: poolView(env.msgBuffer.union(byzSoup), env.blocks) }
    else env

  /// agave patch 7: initializes the environment with the trace's `blocks`.
  /// agave patch 13: and with the pool view of no messages.
  action initWith(blocks: Set[Block]): bool = all {
    s' = withPoolView({
      blocks: blocks,
      pool: { certificates: Set(), parentReady: Set(), safeToNotar: Map(), safeToSkip: Map() },
      system: benevolent.mapBy(_ => {
        // preload state for `numSlots` slots (agave patch 3)
        state: range(0, numSlots).listMap(i => if (i == 0) Set(ParentReady(-1)) else Set()),
        // agave patch 1: one list of pending blocks per slot
        pendingBlocks: range(0, numSlots).listMap(_ => []),
      }),
      msgBuffer: Set(),
      // agave patch 4: `ParentReady(-1)` at slot 0 starts the timeouts of its whole window,
      // as `ParentReadyInput` does via `setTimeouts` (upstream: `Set(0)`).
      activeTimeouts: benevolent.mapBy(v => windowSlots(0)),
    }),
    ch' = aliveSlots.mapBy(s => Set()),
    counter' = 0,
  }

  /// agave patch 7: the trace's blocks are the instance's `allBlocks`. The singleton pick
  /// records them in `mbt::nondetPicks`, which is how the model-based tests learn them.
  action init = {
    nondet blocks = Set(allBlocks).oneOf()
    initWith(blocks)
  }

  //
  // agave patch 8: a block generator, for instances that draw a different block tree per trace.
  // Everything below is a pure function of `initGenerated`'s picks.
  //

  /// agave patch 8: the hash of the block of `slot` in `lane`. Lane 0 is the slot's primary
  /// block, lane 1 an equivocating second block, and lane 9 a phantom parent that is never a
  /// block itself. Every generated hash is distinct and encodes its slot.
  pure def genHash(slot: Slot, lane: int): Blockhash = 100 + 10 * slot + lane

  /// agave patch 8: the hashes of the primary blocks of the latest and the second-latest slot
  /// before `slot` that have one, or -1 (the genesis parent) where there is none.
  pure def latestPrimaries(kind: Slot -> int, slot: Slot): (Blockhash, Blockhash) =
    range(0, slot).foldl((-1, -1), (acc, earlier) =>
      if (kind.keys().contains(earlier) and kind.get(earlier) != 6) (genHash(earlier, 0), acc._1)
      else acc
    )

  /// agave patch 8: the parent of the primary block of `slot`, by `kind.get(slot)`:
  /// - 0..5, honest: the primary block of the latest earlier slot that has one (or genesis).
  ///   At the first slot of a later window, the leader builds on its `ParentReady`, which is
  ///   often a block early in the previous window whose later slots were skipped. So there the
  ///   parent is the latest primary block up to the previous window's slot `k % 4`: its first
  ///   slot for 0 and 4, its second for 1 and 5, its third for 2, and its last for 3;
  /// - 7, phantom: `genHash(slot - 1, 9)`, which is never a block (genesis at slot 0);
  /// - 8, far back: the primary block of the second-latest earlier slot that has one (or genesis);
  /// - 9, genesis.
  /// Kind 6 is a crashed leader: the slot has no block.
  pure def primaryParent(kind: Slot -> int, slot: Slot): Blockhash =
    val k = kind.get(slot)
    val latest = latestPrimaries(kind, slot)
    if (k <= 5) {
      if (slot > 0 and firstSlotInLeaderWindow(slot)) latestPrimaries(kind, slot - 3 + k % 4)._1
      else latest._1
    }
    else if (k == 7) (if (slot == 0) -1 else genHash(slot - 1, 9))
    else if (k == 8) latest._2
    else -1

  /// agave patch 8: the block tree drawn by `initGenerated`. Every slot of `kind` whose leader
  /// did not crash has a primary block (see `primaryParent`). By `equiv.get(slot)`, it also has
  /// an equivocating block with the same parent (0), or with a different parent (1): the honest
  /// one if the primary's is not, and otherwise the far-back one.
  pure def generateBlocks(kind: Slot -> int, equiv: Slot -> int): Set[Block] =
    kind.keys().filter(slot => kind.get(slot) != 6).map(slot => {
      val parent = primaryParent(kind, slot)
      val latest = latestPrimaries(kind, slot)
      val primary = { slot: slot, hash: genHash(slot, 0), parent: parent }
      val second = { slot: slot, hash: genHash(slot, 1), parent: parent }
      val e = equiv.get(slot)
      if (e == 0) Set(primary, second)
      else if (e == 1) Set(primary, { ...second, parent: if (parent == latest._1) latest._2 else latest._1 })
      else Set(primary)
    }).flatten()

  /// agave patch 8: an `init` that draws the trace's blocks with `generateBlocks`. `kind` and
  /// `equiv` weight their choices through the size of their ranges, since `oneOf` is uniform:
  /// per slot, a 6 in 10 honest parent, 1 in 10 each for a crashed leader, a phantom, a far-back,
  /// and a genesis parent, and 1 in 8 each for the two kinds of equivocation. The singleton
  /// `blocks` pick records the tree in `mbt::nondetPicks`, as `init`'s does.
  action initGenerated = {
    nondet kind = setOfMaps(aliveSlots, 0.to(9)).oneOf()
    nondet equiv = setOfMaps(aliveSlots, 0.to(7)).oneOf()
    nondet blocks = Set(generateBlocks(kind, equiv)).oneOf()
    initWith(blocks)
  }

  // agave patch 7: this and the actions below read `s.blocks` (upstream: `allBlocks`).
  action receiveSpecificBlock(v: ProcessID, b: Block): bool = all {
    s.blocks.contains(b),
    v.processInput(BlockInput(b)),
  }

  action receiveBlock(v: ProcessID, now: Slot): bool = all{
    nondet block = s.blocks.filter(b => b.slot == now).oneOf()
    receiveSpecificBlock(v, block)
  }

  /// Fire timeout for slot
  action fireTimeoutEvent(v: ProcessID, now: Slot): bool = all {
    s.activeTimeouts.get(v).contains(now),
    val s2 = { ...s, activeTimeouts: s.activeTimeouts.set(v, s.activeTimeouts.get(v).exclude(Set(now))) }
    // agave patch 13: also updates the pool view.
    s' = withPoolView(applyEffect(s2, v, consensus(s2.system.get(v), TimeOutInput(now)))),
    ch' = finalizedBlocks,
    counter' = counter + 1,
  }

  //
  // auxiliary functions for actions below
  //

  action processInput(id: ProcessID, input: ConsensusInput): bool = all {
    // agave patch 13: also updates the pool view.
    s' = withPoolView(applyEffect(s, id, consensus(s.system.get(id), input))),
    ch' = finalizedBlocks,
    counter' = counter + 1,
  }

  // Definition 15 (Pool events). The following events are emitted as input for
  // Algorithm 1:

  // • BlockNotarized(slot(b), hash(b)): Pool holds a notarization certificate for
  // block b.
  def notarizedBlocksCondition(v: ProcessID, b: BlockReference): bool =
    isCertified(NotarizationCertificate(b), allMessages)

  /// Emit BlockNotarized and process it. The structure of this actions is that we first
  /// build a set of data (blocks or slot numbers) for which a certain event is enabled,
  /// and then we pick one non-deterministically, process it, and updated the global state.
  action blockNotarizedAction(v: ProcessID, now: Slot): bool =
    val bs = s.blocks.filter(b =>
      notarizedBlocksCondition(v, { slot: now, hash: b.hash })
    )
    all {
      bs.size() > 0,
      nondet b = bs.oneOf()
      v.processInput(BlockNotarizedInput({ slot: b.slot, hash: b.hash })),
    }

  // • ParentReady(s, hash(b)): Slot s is the first of its leader window, and Pool
  // holds a notarization or notar-fallback certificate for a previous block b,
  // and skip certificates for every slot s′ since b, i.e., for slot(b) < s′ < s.
  // agave patch 7: takes the environment's `blocks` (upstream: `allBlocks`).
  // agave patch 14: also returns `genesisBlock` if every slot before `slot` is skip-certified.
  pure def parentReadyBlocks(slot: Slot, msgs: Set[NetworkMsg], blocks: Set[Block]): Set[Block] = {
    if (not(firstSlotInLeaderWindow(slot))) Set() else
    val certified = blocks.filter(b =>
      and {
        b.slot < slot,
        or {
          isCertified(NotarizationCertificate(b.reference()), msgs),
          isCertified(NotarFallbackCertificate(b.reference()), msgs)
        },
        // agave patch 2: only the slots strictly between `b` and `slot`, as in the comment
        // above (upstream: `b.slot.to(slot)`, which also requires skip certificates for
        // `b.slot` and `slot`).
        (b.slot + 1).to(slot - 1).subseteq(slotsSkipCertified(msgs))
      }
    )
    // agave patch 14: the genesis parent (hash -1) is certified by definition, as `init`'s
    // `ParentReady(-1)` at slot 0 assumes, so a later window start is ready for it once every
    // earlier slot is skip-certified (upstream: never, since genesis is not one of the blocks).
    // Slot 0 itself is left out, because `init` already made it ready for genesis.
    if (slot > 0 and 0.to(slot - 1).subseteq(slotsSkipCertified(msgs)))
      certified.union(Set(genesisBlock))
    else
      certified
  }

  /// agave patch 14: the genesis block, the parent of every block whose parent hash is -1. It is
  /// never one of the environment's blocks.
  pure val genesisBlock: Block = { slot: -1, hash: -1, parent: -1 }

  /// Emit ParentReady and process it
  action parentReadyAction(v: ProcessID, now: Slot): bool =
    val bs = parentReadyBlocks(now, allMessages, s.blocks)
    all {
      bs.size() > 0,
      nondet b = bs.oneOf()
      // agave patch 2: the input is `ParentReady(s, hash(b))` for the slot `s = now` that
      // became ready (upstream: `b.reference()`, which records the parent at `b.slot` instead).
      v.processInput(ParentReadyInput({ slot: now, hash: b.hash })),
    }

  /// Definition 16 (fallback events)

  pure def votedToSkip(s: Slot, msgs: Set[NetworkMsg]): Set[ProcessID] =
    msgs.filter(m =>  m.msg == SkipVoteMsg(s)).map(m => m.sender)

  // Warning: vote doesn't contain parent. Assumption about parent hashed in hash?
  pure def votedToNotar(b: BlockReference, msgs: Set[NetworkMsg]): Set[ProcessID] =
    msgs.filter(m => m.msg == NotarVoteMsg(b)).map(m => m.sender)

  // • SafeToNotar(s, hash(b)): The event is only issued if the node voted in
  // slot s already, but not to notarize b.
  def safeToNotarCondition(v: ProcessID, now: Slot, b: Block): bool = all {
    // Consider block b in slot s = slot(b).
    b.slot == now,
    safeToNotarHolds(v, b, allMessages, parentNotarFallbackCertified(b, allMessages, s.blocks)),
  }

  /// agave patch 13: the body of `safeToNotarCondition` over the messages `msgs` (upstream: over
  /// `allMessages`, inline), so that `poolView` can evaluate it. Only the messages of `b`'s slot
  /// matter. `parentCertified` is `parentNotarFallbackCertified(b, ...)` (agave patch 16).
  pure def safeToNotarHolds(v: ProcessID, b: Block, msgs: Set[NetworkMsg], parentCertified: bool): bool = all {
    val now = b.slot
    val voted = msgs.filter(m => m.sender == v and now == slotOf(m.msg))
    // agave patch 15: only a notarization vote for `b` itself rules the event out, as the comment
    // above, the paper, and agave say (upstream: any `NotarVoteMsg` in the slot, so a process that
    // voted to notarize another block of the slot never voted to notar-fallback `b`).
    val notarVoted =
      voted.filter(m => match m.msg {
        | NotarVoteMsg(r) => r == b.reference()
        | _ => false
      })

    and {
      voted.size() > 0,
      notarVoted.size() == 0,
    },
    val notar = votedToNotar(b.reference(), msgs)
    val skip = votedToSkip(b.slot, msgs)
    // notar(b) ≥ 40% or (skip(s) + notar(b) ≥ 60% and notar(b) ≥ 20%).
    // The paper postulates that the pool stores for each sender and slot a skip or a notar message
    // but not both. Thus, in the paper they can sum up the sizes of the sets. In this spec we
    // abstract the pool so that here we need to build the union
    // of the skip and notar sets and then compute the size
    or {
      surpassesThreshold(notar, 40),
      and {
        surpassesThreshold(skip.union(notar), 60),
        surpassesThreshold(notar, 20)
      }
    },
    // agave patch 16: outside the first slot of a leader window, the event also waits for a
    // notar-fallback certificate on the parent of `b` (upstream: the parent is ignored). The
    // paper's Definition 16 retrieves `b` through repair to learn its parent, and emits the event
    // once Pool also holds the parent's notar-fallback certificate; agave does the same.
    firstSlotInLeaderWindow(b.slot) or parentCertified,
  }

  /// agave patch 16: whether the parent of `b` has a notar-fallback certificate over `msgs`,
  /// which a notarization or fast-finalization certificate implies. The genesis parent (hash -1)
  /// counts as certified, as `init`'s `ParentReady(-1)` and patch 14 assume. A parent that is not
  /// one of the `blocks` has no certificate, since no correct process votes for it.
  pure def parentNotarFallbackCertified(b: Block, msgs: Set[NetworkMsg], blocks: Set[Block]): bool =
    b.parent == -1 or blocks.exists(p =>
      p.hash == b.parent and isCertified(NotarFallbackCertificate(p.reference()), msgs))

  // Emit SafeToNotar and process it
  action safeToNotarAction(v: ProcessID, now: Slot): bool =
    val bs = s.blocks.filter(b => safeToNotarCondition(v, now, b))
    all {
      bs.size() > 0,
      nondet b = bs.oneOf()
      v.processInput(SafeToNotarInput({ slot: b.slot, hash: b.hash })),
    }

  // • SafeToSkip(s): The event is only issued if the node voted in slot s already,
  // but not to skip s.
  def safeToSkipCondition(v: ProcessID, now: Slot): bool = safeToSkipHolds(v, now, allMessages)

  /// agave patch 13: the body of `safeToSkipCondition` over the messages `msgs` (upstream: over
  /// `allMessages`, inline), so that `poolView` can evaluate it.
  pure def safeToSkipHolds(v: ProcessID, now: Slot, msgs: Set[NetworkMsg]): bool = all {
    // node voted in slot s already, but not to skip s.
    // agave patch 17: `v` sent some message for `now`, and no `SkipVoteMsg(now)` (upstream: some
    // message for `now` other than `SkipVoteMsg(now)`, which also holds after `v` voted to skip
    // `now` and then cast a notar-fallback vote in it).
    msgs.exists(m => m.sender == v and slotOf(m.msg) == now),
    not(msgs.exists(m => m.sender == v and m.msg == SkipVoteMsg(now))),

    // Moreover:
    // skip(s) + Σ_b notar(b) − max_b notar(b) ≥ 40%.
    val blocksWithNotarVotes =
      msgs
        .filter(m => slotOf(m.msg) == now)
        .fold(Set(), (s, m) => match m.msg {
          | NotarVoteMsg(n) => s.setAdd(n)
          | _ => s
        })

    // Σ_b notar(b)
    val allNotarVotes = blocksWithNotarVotes.map(b =>
      votedToNotar(b, msgs)
    ).flatten()

    // max_b
    val blockWithMostVotes = blocksWithNotarVotes.fold({ slot: -1, hash: -1 }, (res, x) => {
      if (votedToNotar(x, msgs).size() > votedToNotar(res, msgs).size())
        x
      else
        res
    })

    // skip(s) + Σ_b notar(b) − max_b notar(b) ≥ 40%.
    // Question: is (+) here the sum of the sizes or the size of the union?
    // same for (-)

    // skip(s)
    votedToSkip(now, msgs)
      // + Σ_b notar(b)
      .union(allNotarVotes)
      // - max_b notar(b)
      .exclude(votedToNotar(blockWithMostVotes, msgs))
      // ≥ 40%.
      .surpassesThreshold(40)
  }

  // Emit SafeToSkip and process it
  action safeToSkipAction(v: ProcessID, now: Slot): bool = all {
    safeToSkipCondition(v, now),
    v.processInput(SafeToSkipInput(now)),
  }

  /// Any upon that is triggered by messages
  action messageResponse (v, slot) =
    any {
      receiveBlock(v, slot),
      blockNotarizedAction(v, slot),
      parentReadyAction(v, slot),
      safeToNotarAction(v, slot),
      safeToSkipAction(v, slot),
    }


  /// agave patch 9: the slot where a process makes progress next: its lowest alive slot without
  /// a vote. All alive slots once the process voted in every one of them.
  pure def frontierSlots(ls: LocalState): Set[Slot] =
    val unvoted = aliveSlots.filter(slot => not(Voted.in(ls.state[slot])))
    if (unvoted.size() == 0) aliveSlots else Set(unvoted.fold(numSlots, (m, x) => min(m, x)))

  /// agave patch 9: whether delivering a block of `slot` to a process in local state `ls` can
  /// only repeat an earlier delivery: the process voted in `slot`, so `consensus` ignores the
  /// block, or every block of `slot` is already pending, so it would be appended again.
  def redundantDelivery(ls: LocalState, slot: Slot): bool = or {
    Voted.in(ls.state[slot]),
    s.blocks.filter(b => b.slot == slot).forall(b =>
      ls.pendingBlocks[slot].foldl(false, (found, p) => found or p == b)),
  }

  /// Main step action. Includes all behavior
  /// agave patch 9: biases the scheduler towards steps that make progress. Upstream bounds
  /// traces with `counter < 10`, picks `slot` uniformly from `aliveSlots`, and responds to a
  /// message (`messageResponse`) as often as it fires a timeout. Here:
  /// - the trace length is left to `--max-steps`;
  /// - `slot` is `v`'s frontier slot in half of the steps;
  /// - a timeout may only fire in one step in four;
  /// - a redundant block delivery may only happen in one step in four.
  /// The branches of `messageResponse` are inlined to gate `receiveBlock`. `any` picks among the
  /// enabled branches, so the gating makes these steps rarer without disabling the step, and
  /// every interleaving stays possible.
  action step = all {
    nondet v = oneOf(benevolent)
    nondet slotCoin = 0.to(1).oneOf()
    nondet slot = oneOf(if (slotCoin == 0) aliveSlots else frontierSlots(s.system.get(v)))
    nondet timeoutCoin = 0.to(3).oneOf()
    nondet redeliveryCoin = 0.to(3).oneOf()
    any {
      any {
        all {
          redeliveryCoin == 0 or not(redundantDelivery(s.system.get(v), slot)),
          receiveBlock(v, slot),
        },
        blockNotarizedAction(v, slot),
        parentReadyAction(v, slot),
        safeToNotarAction(v, slot),
        safeToSkipAction(v, slot),
      },
      all { timeoutCoin == 0, fireTimeoutEvent(v, slot) },
    }
  }

  /// Alternative step action. With this, no timeout ever expires
  action noTimeout = any {
    nondet v = oneOf(benevolent)
    nondet slot = oneOf(aliveSlots)    
    messageResponse(v, slot),
  }



  //
  //
  // Invariants and Witnesses
  //
  //

  // agave patch 7: the invariants and witnesses read `s.blocks` (upstream: `allBlocks`).
  val agreement =
    aliveSlots.forall(slot => finalized(slot, allMessages, s.blocks).size() <= 1)

  val fastAgreement =
    aliveSlots.forall(slot =>
      fastFinalized(slot, allMessages, s.blocks)
        .union(finalized(slot, allMessages, s.blocks))
        .size() <= 1)

  // Witness was used to find bug in safeToNotar encoding
  val noBad =
    benevolent.forall(v => not(s.system.get(v).state[1].contains(BadWindow)))

  /// Witness to find a run where a block is fast finalized
  val fastFinalizedWitness =
    s.blocks.forall(b => not(isCertified(FastFinalizationCertificate(b.reference()), allMessages)))

  /// Witness to find a run where a timeout is started
  val timeoutWitness = 
     s.activeTimeouts.values().forall(v => v == Set())

  /// Theorem 1 (safety). If any correct node finalizes a block b in slot s and any
  /// correct node finalizes any block b′ in any slot s′ ≥ s, b′ is a descendant of b.
  val safety = tuples(aliveSlots, s.blocks).forall(((slot, block)) => {
    val finalizedAtSlot = finalized(slot, allMessages, s.blocks)
    block.hash.in(finalizedAtSlot) implies tuples(
      aliveSlots.filter(s2 => s2 >= slot), s.blocks).forall(((slot2, block2)) => {
      block2.hash.in(finalized(slot2, allMessages, s.blocks)) implies block2.isDescendant(block, s.blocks)
    })
  })

  /// This generates example traces where a block is finalized after some empty slots
  /// increase max steps to approx 100.
  val emptySlotsWitness = 
    aliveSlots.forall(s1 => 
      aliveSlots.forall(s2 => 
        s1 < s2 implies
      ch.get(s1).size() >= ch.get(s2).size()))
                              
  /// This generates example traces where every slot is finalized
  /// increase max steps to approx 100.
  val noEmptySlotsWitness = 
    aliveSlots.exists(s1 => 
      ch.get(s1).size() == 0)

  // agave patch 9: coverage witnesses for tuning the scheduler without the model-based tests,
  // for example `quint run statemachine.qnt --main=agave_gen --init=initGenerated
  // --witnesses notarizedInWindow1 parentReadySlot4 notarFallbackVoted skipFallbackVoted
  // slotFinalized`. `--witnesses` counts the traces with a state where each one holds.

  /// A benevolent process saw a block notarized in window 1 (slots 4..7).
  val notarizedInWindow1 = benevolent.exists(v =>
    aliveSlots.filter(slot => slot >= 4).exists(slot =>
      s.system.get(v).state[slot].exists(o => match o { | BlockNotarized(_) => true | _ => false })))

  /// A benevolent process has `ParentReady` for slot 4, the first slot of window 1.
  val parentReadySlot4 = benevolent.exists(v =>
    aliveSlots.filter(slot => slot == 4).exists(slot =>
      s.system.get(v).state[slot].exists(o => match o { | ParentReady(_) => true | _ => false })))

  /// A benevolent process cast a notar-fallback vote.
  val notarFallbackVoted =
    s.msgBuffer.exists(m => match m.msg { | NotarFallBackVoteMsg(_) => true | _ => false })

  /// A benevolent process cast a skip-fallback vote.
  val skipFallbackVoted =
    s.msgBuffer.exists(m => match m.msg { | SkipFallbackVoteMsg(_) => true | _ => false })

  /// A slot is finalized, slowly or fast.
  val slotFinalized = aliveSlots.exists(slot =>
    finalized(slot, allMessages, s.blocks).union(fastFinalized(slot, allMessages, s.blocks)).size() > 0)

  // agave patch 19: witnesses of the scenarios that the targeted model-based tests
  // (`mbt_votor_agave_gen_targets` and `mbt_votor_agave_pool_targets`) replay, together with
  // `notarizedInWindow1` and `skipFallbackVoted` above. A target is `not(<witness>)`, passed to
  // `quint run --invariant`: every trace that violates it reaches the scenario, and is cut off at
  // the first state where it does. As for the patch 9 witnesses, no action reads them, and
  // `--witnesses` measures how many traces reach each one (see the target catalogue in
  // `README.md`, which also lists the scenarios left out).

  /// A benevolent process has `ParentReady` for slot 4 with a parent before slot 3, or with
  /// genesis, so that at least slot 3 of window 0 is skip-certified.
  val parentReadySlot4AfterSkip = benevolent.exists(v =>
    aliveSlots.filter(slot => slot == 4).exists(slot =>
      s.system.get(v).state[slot].exists(o => match o {
        | ParentReady(h) => h == -1 or s.blocks.exists(b => b.hash == h and b.slot < 3)
        | _ => false
      })))

  /// A benevolent process cast a notar-fallback vote for a block of a slot in which it voted to
  /// notarize another block, which agave patch 15 allows.
  val notarFallbackForOtherBlock = s.msgBuffer.exists(m => match m.msg {
    | NotarFallBackVoteMsg(r) => s.msgBuffer.exists(n => n.sender == m.sender and (match n.msg {
        | NotarVoteMsg(r2) => r2.slot == r.slot and r2.hash != r.hash
        | _ => false
      }))
    | _ => false
  })

  /// A slot is finalized, slowly or fast, and a benevolent process voted to skip an earlier slot
  /// of the slot's leader window. The skip votes come first because they are cheaper to check.
  val finalizedAfterSkipVote = aliveSlots.exists(slot => and {
    windowSlots(slot).exists(earlier =>
      earlier < slot and s.msgBuffer.exists(m => m.msg == SkipVoteMsg(earlier))),
    finalized(slot, allMessages, s.blocks).union(fastFinalized(slot, allMessages, s.blocks)).size() > 0,
  })

  /// A block is fast-finalized.
  val blockFastFinalized =
    s.blocks.exists(b => isCertified(FastFinalizationCertificate(b.reference()), allMessages))

  /// A benevolent process cast a notar-fallback vote outside the first slot of a leader window,
  /// where `SafeToNotar` waits for the block's parent to be certified (agave patch 16).
  val intraWindowNotarFallbackVoted = s.msgBuffer.exists(m => match m.msg {
    | NotarFallBackVoteMsg(r) => not(firstSlotInLeaderWindow(r.slot))
    | _ => false
  })

  /// A benevolent process has `ParentReady` for genesis at slot 4 (agave patch 14).
  val parentReadyGenesisSlot4 = benevolent.exists(v =>
    aliveSlots.filter(slot => slot == 4).exists(slot =>
      s.system.get(v).state[slot].contains(ParentReady(-1))))

  /// A slot is skip-certified without any skip vote, by skip-fallback votes alone, which agave
  /// patch 18 lets count for `ParentReady`. Too rare for a target (see `README.md`).
  val skipCertifiedByFallbackOnly = aliveSlots.exists(slot => and {
    isCertified(SkipCertificate(slot), allMessages),
    not(allMessages.exists(m => m.msg == SkipVoteMsg(slot))),
  })

  // TODO finalize ancestors locally
}

module some_byz {
  // For a trace where a value is fast finalized call
  // quint run statemachine.qnt --main some_byz --invariant fastFinalizedWitness
  // Same but one process reached ItsOver
  // quint run statemachine.qnt --main some_byz --invariant fastFinalizedWitness  --seed=0x11fa3a6ddd2666   
  import consensus(
    correct = Set("v1", "v2", "v3", "v4", "v5"),
    good = Set(),
    byzantine = Set("b1"),
    power = Set("v1", "v2", "v3", "v4", "v5", "b1").mapBy(_ => 1), 
    correctBlocks = Set(
      { slot: 0, hash: 42, parent: -1 },
      { slot: 1, hash: 43, parent: 42 },
      { slot: 2, hash: 44, parent: 43 },
    ),
    byzantineBlocks = Set(
      { slot: 1, hash: 46, parent: 42 },
      { slot: 1, hash: 47, parent: 45 },
      { slot: 2, hash: 48, parent: 43 },
      { slot: 2, hash: 49, parent: 45 },
    ),
    aliveSlots = 0.to(2),
    aliveHashes = 42.to(49),
  ).*

  run ffTest =
    init
    .then(receiveSpecificBlock("v1", { hash: 42, parent: -1, slot: 0 }))
    .then(receiveSpecificBlock("v2", { hash: 42, parent: -1, slot: 0 }))
    .then(receiveSpecificBlock("v3", { hash: 42, parent: -1, slot: 0 }))
    .then(receiveSpecificBlock("v4", { hash: 42, parent: -1, slot: 0 }))
    .expect(fastFinalized(0, allMessages, s.blocks) == Set(42))

}


module some_byz_vp {
  // For a trace where a value is fast finalized call
  // quint run statemachine.qnt --main some_byz --invariant fastFinalizedWitness
  // Same but one process reached ItsOver
  // quint run statemachine.qnt --main some_byz --invariant fastFinalizedWitness  --seed=0x11fa3a6ddd2666   
  import consensus(
    correct = Set("v1", "v2"),
    good = Set(),
    byzantine = Set("b1"),
    power = Map("v1" -> 3, "v2" -> 2, "b1"-> 1), 
    correctBlocks = Set(
      { slot: 0, hash: 42, parent: -1 },
      { slot: 1, hash: 43, parent: 42 },
      { slot: 2, hash: 44, parent: 43 },
    ),
    byzantineBlocks = Set(
      { slot: 1, hash: 46, parent: 42 },
      { slot: 1, hash: 47, parent: 45 },
      { slot: 2, hash: 48, parent: 43 },
      { slot: 2, hash: 49, parent: 45 },
    ),
    aliveSlots = 0.to(2),
    aliveHashes = 42.to(49),
  ).*

  // run ffTest =
  //   init
  //   .then(receiveSpecificBlock("v1", { hash: 42, parent: -1, slot: 0 }))
  //   .then(receiveSpecificBlock("v2", { hash: 42, parent: -1, slot: 0 }))
  //   .expect(fastFinalized(0, allMessages, s.blocks) == Set(42))

}



module too_many_byz_1 {
  // to violate agreement, run
  // quint run statemachine.qnt --main too_many_byz_1 --step oneSlotNoTimeout  --invariant agreement

  import consensus(
    correct = Set("v1", "v2", "v3", "v4"),
    good = Set(),
    byzantine = Set("b1"),
    power = Set("v1", "v2", "v3", "v4", "b1").mapBy(_ => 1), 
    correctBlocks = Set(
      { slot: 0, hash: 42, parent: -1 },
      { slot: 1, hash: 43, parent: 42 },
      { slot: 2, hash: 44, parent: 43 },
    ),
    byzantineBlocks = Set(
      { slot: 0, hash: 45, parent: -1 },
      { slot: 1, hash: 46, parent: 42 },
      { slot: 1, hash: 47, parent: 45 },
      { slot: 2, hash: 48, parent: 43 },
      { slot: 2, hash: 49, parent: 45 },
    ),
    aliveSlots = 0.to(2),
    aliveHashes = 42.to(49),
  ).*
}

module too_many_byz {
  // to violate agreement, run
  // quint run statemachine.qnt --main too_many_byz --step oneSlotNoTimeout --invariant agreement

  import consensus(
    correct = Set("v1", "v2", "v3", "v4"),
    good = Set(),
    byzantine = Set("b1", "b2"),
    power = Set("v1", "v2", "v3", "v4", "b1", "b2").mapBy(_ => 1), 
    correctBlocks = Set({ slot: 0, hash: 42, parent: -1 }),
    byzantineBlocks = Set({ slot: 0, hash: 41, parent: -1 }),
    aliveSlots = Set(0),
    aliveHashes = 41.to(42),
  ).*

  run disagreementExampleTest =
    init
    .then(receiveSpecificBlock("v3", { hash: 41, parent: -1, slot: 0 }))
    .then(receiveSpecificBlock("v4", { hash: 41, parent: -1, slot: 0 }))
    .expect(s.msgBuffer == 
      Set(  { msg: NotarVoteMsg({ hash: 41, slot: 0 }), sender: "v3" },
            { msg: NotarVoteMsg({ hash: 41, slot: 0 }), sender: "v4" }))
    .then(receiveSpecificBlock("v2", { hash: 42, parent: -1, slot: 0 }))
    .then(blockNotarizedAction("v3", 0))
    .then(blockNotarizedAction("v4", 0)) 
    .expect(
        Set(  { msg: FinalVoteMsg(0), sender: "v3" },
              { msg: FinalVoteMsg(0), sender: "v4" })
        .subseteq(s.msgBuffer))
    .then(receiveSpecificBlock("v1", { hash: 42, parent: -1, slot: 0 }))    
    .expect(not(agreement))
    .expect(
      Set(  { msg: NotarVoteMsg({ hash: 41, slot: 0 }), sender: "v3" },
            { msg: NotarVoteMsg({ hash: 41, slot: 0 }), sender: "v4" },
            { msg: NotarVoteMsg({ hash: 41, slot: 0 }), sender: "b1" },
            { msg: NotarVoteMsg({ hash: 41, slot: 0 }), sender: "b2" }, 
            { msg: NotarVoteMsg({ hash: 42, slot: 0 }), sender: "v1" },
            { msg: NotarVoteMsg({ hash: 42, slot: 0 }), sender: "v2" },
            { msg: NotarVoteMsg({ hash: 42, slot: 0 }), sender: "b1" },
            { msg: NotarVoteMsg({ hash: 42, slot: 0 }), sender: "b2" },
            { msg: FinalVoteMsg(0), sender: "v3" },
            { msg: FinalVoteMsg(0), sender: "v4" },
            { msg: FinalVoteMsg(0), sender: "b1" },
            { msg: FinalVoteMsg(0), sender: "b2" })
      .subseteq(allMessages))
  

}


/// agave-specific instance (not upstream). The upstream instances only have slots 0..2, so
/// slot 0 is their only first slot of a leader window, and `parentReadyAction` can never fire.
/// Here slot 4 starts a second window, whose parent can be block 42 (after slots 1..3 are
/// skipped) or block 43 or 46 (after slots 2..3 are skipped).
module agave_window {
  import consensus(
    correct = Set("v1", "v2"),
    good = Set(),
    byzantine = Set("b1"),
    // v1 and b1 together reach the 60% certificate threshold.
    power = Map("v1" -> 3, "v2" -> 2, "b1" -> 1),
    correctBlocks = Set(
      { slot: 0, hash: 42, parent: -1 },
      { slot: 1, hash: 43, parent: 42 },
      { slot: 4, hash: 50, parent: 42 },
      { slot: 4, hash: 51, parent: 43 },
    ),
    byzantineBlocks = Set(
      { slot: 1, hash: 46, parent: 42 },
      { slot: 4, hash: 52, parent: 45 },
    ),
    aliveSlots = 0.to(4),
    aliveHashes = Set(42, 43, 46, 50, 51, 52),
  ).*
}


/// agave-specific instance (not upstream; agave patch 8). Every trace draws its own block tree
/// over two leader windows (slots 0..7) with `initGenerated`, so it must be run with
/// `--init=initGenerated`. The Byzantine soup covers every hash the generator can use,
/// `genHash(slot, lane)` for every slot and lane 0, 1, and 9, so it does not depend on the tree.
module agave_gen {
  import consensus(
    correct = Set("v1", "v2", "v3"),
    good = Set(),
    byzantine = Set("b1"),
    power = Map("v1" -> 3, "v2" -> 2, "v3" -> 2, "b1" -> 1),
    correctBlocks = Set(),
    byzantineBlocks = Set(),
    aliveSlots = 0.to(7),
    // `genHash(slot, lane)`, which is not in scope here
    aliveHashes = tuples(0.to(7), Set(0, 1, 9)).map(((slot, lane)) => 100 + 10 * slot + lane),
  ).*
}


/// agave-specific instance (not upstream; agave patch 12). `agave_gen` without Byzantine
/// voters, for the model-based tests that run agave's consensus pool next to each event handler:
/// the Byzantine soup lets a Byzantine process send conflicting votes, which the spec counts once
/// per sender and agave's pool once per vote. `agave_gen`'s b1 becomes the correct v4. Byzantine
/// leaders are still covered by the generated block trees. Must be run with
/// `--init=initGenerated`.
module agave_pool {
  import consensus(
    correct = Set("v1", "v2", "v3", "v4"),
    good = Set(),
    byzantine = Set(),
    // Not `agave_gen`'s 3, 2, 2, 1: there, v2 and v3 notarizing a block (4 of 8, 50%) and then
    // v1 (7 of 8) takes the notarize votes from below 60% to 80% at once. Agave's pool then
    // forms only the fast-finalization certificate and emits `Finalized(b, true)` but never
    // `BlockNotarized(b)`, while the spec's `blockNotarizedAction` is enabled. With 3, 2, 2, 2
    // (60% is 6 of 9 and 80% is 8 of 9), no single vote gets from 5 or less to 8 or more.
    power = Map("v1" -> 3, "v2" -> 2, "v3" -> 2, "v4" -> 2),
    correctBlocks = Set(),
    byzantineBlocks = Set(),
    aliveSlots = 0.to(7),
    // `genHash(slot, lane)`, which is not in scope here. With no Byzantine process, this only
    // sizes the (empty) Byzantine soup.
    aliveHashes = tuples(0.to(7), Set(0, 1, 9)).map(((slot, lane)) => 100 + 10 * slot + lane),
  ).*
}
