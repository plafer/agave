# Vendored Alpenglow Quint specification

These files are vendored from the Alpenglow Quint specification so that the
model-based tests in `votor/src/event_handler/mbt` can generate traces
without an external checkout.

- Upstream repository: <https://github.com/informalsystems/Alpenglow-spec>
- Upstream commit: `3da84873de0e5b170168511e3baf5cedb9393544`
  ("Merge pull request #2 from informalsystems/josef-no-blue", 2026-06-17)
- Vendored files: `alpenglow.qnt`, `statemachine.qnt`, `basicSpells.qnt`

`lemmas.qnt` is not vendored: it is not imported by the state machine and it
does not typecheck at the upstream commit.

## Patches

Every change to the vendored files is listed here, with the reason for it.
Each changed spot is marked with an `agave patch N` comment. Patches 1, 2,
4, 5, 6, 11, 14, 15, 16, 17, and 18 are to be reported upstream. Patches 17
and 18 were applied before 15 and 16, which kept the numbers planned for them.

### Patch 1: pending blocks are a per-slot list, tried in arrival order

`alpenglow.qnt`, `statemachine.qnt` (`init`).

Upstream keeps at most one pending block per slot (`List[Option[Block]]`),
and `setPendingBlock` overwrites it, so `checkPendingBlocks` tries the
*latest* block received for a slot. Agave keeps every pending block of a slot
in arrival order and tries them in that order, so the *earliest* eligible
block wins. This difference is reachable in `some_byz` (blocks 43 and 46 are
both in slot 1 with parent 42). The patch makes `pendingBlocks` a
`List[List[Block]]`, replaces `setPendingBlock` with `appendPendingBlock` and
`clearPendingBlocks`, and has `checkPendingBlocks` try each slot's blocks in
order.

### Patch 2: `parentReadyAction` fixes

`statemachine.qnt` (`parentReadyBlocks`, `parentReadyAction`).

Both bugs are latent upstream, because the first one makes
`parentReadyAction` unreachable in practice.

- `parentReadyBlocks` required skip certificates for `b.slot.to(slot)`, which
  includes `b.slot` itself (where `b` is notarized) and `slot` (which has not
  started). The comment above it, and agave's `ParentReadyTracker`, only
  require them for the slots strictly between the two. The patch uses
  `(b.slot + 1).to(slot - 1)`.
- `parentReadyAction` passed `ParentReadyInput(b.reference())`, so
  `consensus` recorded `ParentReady(b.hash)` at `b.slot` (the parent's slot)
  and scheduled the timeouts of the parent's window, instead of recording it
  at the slot `now` that became ready. The patch passes
  `{ slot: now, hash: b.hash }`, matching the paper's `ParentReady(s, hash(b))`.

### Patch 3: per-process state covers every window with an alive slot

`statemachine.qnt` (`numSlots`, `init`).

Upstream preloads a fixed 5 slots of state and pending blocks. The
`agave_window` instance has slot 4 alive, and `trySkipWindow` on it touches
slots 4..7. The patch sizes both lists to `numSlots`, which covers
`windowSlots(slot)` for every alive slot. For the upstream instances
(`aliveSlots = 0.to(2)`) this is 4 entries instead of 5; slot 4 was never
used there.

### Patch 4: the initial `ParentReady` starts timeouts for its whole window

`statemachine.qnt` (`init`).

Upstream `init` preloads `ParentReady(-1)` at slot 0 but only activates the
timeout of slot 0, while `ParentReadyInput` (via `setTimeouts`) activates the
timeouts of the whole window. Agave's initial `ParentReady` also arms its
whole window. The patch activates `windowSlots(0)`.

This is needed for reachability: a correct process that votes to notarize a
block in slot 0 can otherwise never skip slots 1..3, so in `agave_window`
(where v1's vote is needed for every certificate) slot 4 almost never becomes
parent-ready. Measured over 200 traces with a fixed seed, the patch raises
the traces that reach `parentReadyAction` from 1 to 15. It also changes the
mix of actions in the upstream instances (more `fireTimeoutEvent`, fewer
`receiveBlock`).

### Patch 5: fallback votes are recorded and cast at most once

`alpenglow.qnt` (`SlotObject`, `consensus` on `SafeToNotarInput` and
`SafeToSkipInput`).

Upstream's `SafeToNotarInput(sh)` and `SafeToSkipInput(slot)` only check
`ItsOver` before broadcasting the fallback vote, so a repeated input
broadcasts it again. Agave also skips the vote when its `VoteHistory` records
that it already voted notar-fallback for that block
(`voted_notar_fallback(slot, block_id)`) or skip-fallback for that slot
(`voted_skip_fallback(slot)`). The patch adds the `SlotObject` variants
`VotedNotarFallback(Blockhash)` and `VotedSkipFallback`, requires their
absence before broadcasting, and adds them (together with `BadWindow`) when
the vote is cast. The model-based tests compare both flags with agave's
`VoteHistory`.

### Patch 6: `tryFinal` requires `not(ItsOver)`

`alpenglow.qnt` (`tryFinal`).

Upstream's `tryFinal` broadcasts `FinalVoteMsg(slot)` whenever the block is
notarized, voted on, and the window is not bad, so every repeated
`BlockNotarizedInput` broadcasts it again. Agave's `try_final` also requires
`!its_over(slot)`, so it votes to finalize at most once. The patch adds the
same condition.

Neither patch changes the set of broadcast messages, since `msgBuffer` is a
set. With both, a correct process never broadcasts the same message twice,
which the model-based tests check on the agave side.

### Patch 7: the block set is part of the environment

`alpenglow.qnt` (`isDescendant`, `finalized`, `fastFinalized`),
`statemachine.qnt` (`Environment`, `init`, the actions, the invariants and
witnesses).

Not a fix, and not to be reported upstream. Upstream reads the constant
`allBlocks` wherever it needs the blocks. The patch adds a `blocks` field to
`Environment`, set once by the new `initWith(blocks)` action, and has
`receiveSpecificBlock`, `receiveBlock`, `blockNotarizedAction`,
`parentReadyBlocks`, `safeToNotarAction`, `finalizedBlocks`, and the
invariants and witnesses read `s.blocks`. The three pure helpers in
`alpenglow.qnt` that fold over blocks take the block set as an extra
parameter. `consensus` is untouched, and `applyEffect` spreads the rest of
the environment, so no action's assignments change.

`init` is now `nondet blocks = Set(allBlocks).oneOf()` followed by
`initWith(blocks)`, so every instance keeps its fixed blocks and its traces.
The singleton pick records the block set in the `init` state's
`mbt::nondetPicks`. The model-based tests cannot read the ITF state itself,
so this pick is how the driver learns each trace's blocks, and it no longer
keeps its own copy of them. This prepares for instances that generate a
different block set per trace.

### Patch 8: a block generator and the `agave_gen` instance

`statemachine.qnt` (`genHash`, `latestPrimaries`, `primaryParent`,
`generateBlocks`, `initGenerated`, module `agave_gen`).

Not a fix, and not to be reported upstream. `initGenerated` is an alternative
`init` that draws a different block tree for every trace. It picks two maps
from slot to int, `kind` and `equiv`, and `generateBlocks(kind, equiv)`, a
pure function of the two, turns them into the block set passed to
`initWith`. `oneOf` is uniform, so the size of each range sets the weights.
Per slot:

- `kind` in `0..9`: 0 to 5 give the primary block an honest parent (the
  primary block of the latest earlier slot that has one), 6 is a crashed
  leader (no block), 7 a phantom parent that is never a block, 8 a parent
  further back (the primary block of the second-latest earlier slot that has
  one), and 9 the genesis parent. At the first slot of a window other than
  window 0, the honest parent is instead the latest primary block up to slot
  `k % 4` of the previous window (its first slot for 0 and 4, its second for
  1 and 5, its third for 2, its last for 3). An honest leader builds on its
  `ParentReady`, which is often a block early in the previous window whose
  later slots were skipped. With "the latest earlier slot" only, the parent of
  a window-1 block rarely matched a `ParentReady` of slot 4: with the patch 9
  scheduler and 60 steps, 5 to 7% of 200 traces notarized a window-1 block
  (the `notarizedInWindow1` witness), against 14 to 20% with this rule.
- `equiv` in `0..7`: 0 adds an equivocating block with the same parent as the
  primary block, 1 one with a different parent, and 2 to 7 nothing.

Hashes encode their slot: `genHash(slot, lane) = 100 + 10 * slot + lane`,
with lane 0 for the primary block, 1 for the equivocating block, and 9 for
phantom parents. The resulting trees have forks, equivocations, phantom,
far-back, and genesis parents, and empty slots. Parents are fixed before the
trace starts, so an "honest" parent only approximates what an honest leader
would choose from its own `ParentReady`.

`agave_gen` has three correct processes (stakes 3, 2, 2), one Byzantine
process (stake 1), and slots 0..7, so slot 4 starts a second leader window.
Its `correctBlocks` and `byzantineBlocks` are empty, so it must be run with
`--init=initGenerated`. Its `aliveHashes` are every hash the generator can
use, so the Byzantine message soup does not depend on the trace's tree. The
`blocks` pick is still how the model-based tests learn the tree; they never
read `kind` or `equiv`. `consensus` in `alpenglow.qnt` is untouched.

### Patch 9: a scheduler biased towards progress, without a step bound

`statemachine.qnt` (`frontierSlots`, `redundantDelivery`, `step`, and the
witnesses `notarizedInWindow1`, `parentReadySlot4`, `notarFallbackVoted`,
`skipFallbackVoted`, `slotFinalized`).

Not a fix, and not to be reported upstream. Upstream's `step` requires
`counter < 10`, picks `slot` uniformly from `aliveSlots`, and fires a timeout
as often as it responds to a message. With generated trees over two windows,
traces then rarely leave window 0: nodes skip it before two of them vote for
the same block, and most steps deliver a block that the node already has or
cannot use. The patch changes `step` only:

- There is no bound on `counter`; each test's `max_steps` sets the trace
  length.
- In half of the steps, `slot` is the lowest alive slot in which `v` has not
  voted (`frontierSlots`), and a uniform alive slot otherwise.
- A timeout may only fire in one step in four.
- A block delivery that can only repeat an earlier one (`v` voted in the slot,
  or every block of the slot is already pending at `v`) may only happen in one
  step in four. `step` inlines the branches of `messageResponse` to gate
  `receiveBlock`.

`any` picks among the enabled branches, so the gating makes these steps rarer
without disabling `step`, and every interleaving stays possible. `consensus`,
the actions, and `noTimeout` are unchanged. The change applies to every
instance. The witnesses measure the scheduler without the model-based tests,
for example `quint run statemachine.qnt --main=agave_gen --init=initGenerated
--max-steps=60 --witnesses notarizedInWindow1 parentReadySlot4
notarFallbackVoted skipFallbackVoted slotFinalized`.

### Patch 10: the Byzantine soup skips votes for other slots' generated hashes

`statemachine.qnt` (`isGenHashOfOtherSlot`, `byzSoup`, `allMessages`).

Not a fix, and not to be reported upstream. `byzNetworkMsgs` pairs every alive
slot with every alive hash, so in `agave_gen` it has Byzantine notar and
notar-fallback votes for 192 block references, of which only the 24 whose
`genHash` (patch 8) encodes their own slot can name a block. `allMessages`
now reads `byzSoup`, which leaves out the other 168. No certificate or
condition on a trace block changes, since each slot keeps the Byzantine votes
for its own three hashes (this also keeps `safeToSkipCondition` unchanged).
The generated traces are identical with and without the patch (checked on
several seeds), and `quint run` on `agave_gen` is about three times faster.
For the fixed instances, whose hashes are below 100, `byzSoup` is
`byzNetworkMsgs`.

### Patch 11: `isDescendant` walks the slots in ascending order

`alpenglow.qnt` (`isDescendant`).

`isDescendant(a, b, blocks)` collects the descendants of `b` slot by slot:
a block of slot `s` joins the path if its parent is already in it. That only
works if the slots are visited in ascending order, but upstream folds over
the set `b.slot.to(a.slot)`, and `quint run`'s Rust backend folds over a set
in no particular order (`0.to(9).fold([], (l, e) => l.append(e))` gives
`[0, 8, 5, 2, 7, 4, 1, 9, 6, 3]` on quint 0.33.0). So `isDescendant` could
miss a descendant more than one block away, and `--invariant=safety` failed on
`agave_gen` traces that finalize a block and its grandchild (for example
100 in slot 0 and 150 in slot 5, via 140 in slot 4). The patch folds over the
list `range(b.slot, a.slot + 1)` with `foldl`. `safety` is only an
invariant, and neither `consensus` nor the model-based tests read
`isDescendant`, so the patch changes no trace. The bug is upstream too,
wherever a block and a descendant two or more blocks down are both finalized,
and is to be reported upstream.

### Patch 12: the `agave_pool` instance

`statemachine.qnt` (module `agave_pool`).

Not a fix, and not to be reported upstream. `agave_pool` is `agave_gen`
without Byzantine processes: its b1 becomes the correct v4, with four correct
processes of stakes 3, 2, 2, and 2, and slots 0..7. It must be run with
`--init=initGenerated`. The model-based tests run agave's consensus pool for
every process in this instance. With a Byzantine process, the Byzantine soup
would let it send conflicting votes, which the spec counts once per sender and
agave's pool once per vote (in production, bls-sigverify drops such votes
before they reach the pool). Byzantine leaders are still covered by the
generated block trees.

v4 has stake 2 rather than `agave_gen`'s 1 for b1. With 3, 2, 2, 1, v2 and v3
notarizing a block (4 of 8, 50%) and then v1 (7 of 8) takes the notarize
votes from below 60% to 80% in one vote. Agave's pool then only forms the
fast-finalization certificate, and never emits `BlockNotarized` for the
block, while the spec's `blockNotarizedAction` is enabled. With 3, 2, 2, 2,
60% is 6 of 9 and 80% is 8 of 9, and no single vote gets from 5 or less to 8
or more.

### Patch 13: the environment has a pool view

`statemachine.qnt` (`PoolView`, `Environment`, `poolView`, `withPoolView`,
`initWith`, `fireTimeoutEvent`, `processInput`, `safeToNotarHolds`,
`safeToSkipHolds`).

Not a fix, and not to be reported upstream. The model-based tests compare
agave's consensus pools with the spec, but ITF traces only record state
variables, and the spec's certificates are conditions over the message soup.
The patch adds a `pool` field to `Environment` that `initWith`,
`fireTimeoutEvent`, and `processInput` recompute after the step's effect.
`poolView` holds every notarization, notar-fallback, and fast-finalization
certificate on one of the trace's blocks, and every skip and finalization
certificate on an alive slot, for which `isCertified` holds over
`msgBuffer` and the Byzantine soup, and every pair `(s, hash(b))` with `b` in
`parentReadyBlocks(s, ...)`, plus the `(0, -1)` that `init` preloads. For
every benevolent process, it also holds the blocks for which
`safeToNotarCondition` holds and the slots for which `safeToSkipCondition`
holds; unlike the rest, these depend on the process's own votes. To evaluate
them, the bodies of the two conditions moved into the pure
`safeToNotarHolds` and `safeToSkipHolds`, which take the messages as a
parameter. Each candidate certificate and each condition is checked against
the messages of its own slot only (plus, for `SafeToNotar`, the parent's
certificate, patch 16), which gives the same result faster.

No action reads the field, so no trace changes apart from it: with and without
the patch, `quint run --mbt` produces the same actions, picks, and states
(ignoring `pool`) for 50 traces of each of the five instances, at seeds
`0x1234` and `0x4321`. The same check passes with the safe-to sets added and
the conditions moved into pure definitions (without patches 15 and 16). The
view is only computed in instances without Byzantine processes, the ones where
the model-based tests run agave's pools (patch 12). Elsewhere it stays empty, because evaluating it over the Byzantine
soup made `quint run` on `agave_gen` take about 28 s instead of 11 s for 180
traces.

### Patch 14: genesis is a parent-ready candidate

`statemachine.qnt` (`parentReadyBlocks`, `genesisBlock`).

`init` preloads `ParentReady(-1)` at slot 0, so the genesis parent (hash -1)
is treated as certified. But `parentReadyBlocks` only returns blocks of the
environment, and genesis is never one of them, so upstream a later window
start can never become ready for genesis, even if every earlier slot is
skip-certified. A fully skipped first window then leaves no parent at all.
Agave's `ParentReadyTracker` starts from the same initial parent ready by
marking the slots before it skip-certified, so its pools emit
`ParentReady(4, genesis)` (in spec slots) once spec slots 0 to 3 have skip
certificates. With the pool view (patch 13), every run of `agave_pool`
failed on this, for example with `pool.parentReady: only in agave: (4, -1)`
(`QUINT_SEED=0xbfa8f22b`). The patch has `parentReadyBlocks(slot, ...)` also
return `genesisBlock = { slot: -1, hash: -1, parent: -1 }` when `slot > 0`
and every slot in `0..slot - 1` is skip-certified. Slot 0 is left out because
`init` already made it ready for genesis, and firing it again would restart
its timeouts. This changes traces of `agave_window`, `agave_gen`, and
`agave_pool`: at seed `0x1234`, `parentReadyAction` for genesis occurs in 16,
8, and 11 of 50 traces. The upstream instances have no second window, so
their traces do not change. The gap is latent upstream for the same reason.

### Patch 15: `safeToNotarCondition` only rules out a notarize vote for the block itself

`statemachine.qnt` (`safeToNotarHolds`, the body of `safeToNotarCondition`).

The paper (Definition 16) issues `SafeToNotar(s, hash(b))` only "if the node
voted in slot s already, but not to notarize b", and the comment in the spec
says the same. Upstream requires that the process sent no `NotarVoteMsg` at
all in the slot, so a process that voted to notarize another block of the slot
never casts a notar-fallback vote for `b`, however many votes `b` gets. Agave's
pool emits `SafeToNotar(b)` unless the node's first vote in the slot was a
notarize vote for `b` itself. With the safe-to sets in the pool view (patch
13), runs of `agave_pool` failed this way, for example with
`pool.safeToNotar[v4]: only in agave: BlockRef { slot: 0, hash: 100 }`
(`QUINT_SEED=0xd047106b`, and `0x1adadb24`): v4 had voted to notarize 101,
and v1 and v2 had voted for 100, 5 of 9 and so at least 40%. The patch only rules out a
`NotarVoteMsg` for `b`. It changes the condition for every instance.

### Patch 16: outside the first slot of a window, `SafeToNotar` waits for the parent's certificate

`statemachine.qnt` (`safeToNotarHolds`, `parentNotarFallbackCertified`,
`safeToNotarCondition`, `poolView`).

The paper's Definition 16 continues: if `s` is the first slot of its leader
window, the event is emitted; otherwise block `b` is first retrieved by
repair, to learn its parent, and the event is emitted once Pool also holds a
notar-fallback certificate for the parent. Upstream ignores the parent. Agave
does what the paper says: its pool holds such a block back, and the consensus
pool service emits `SafeToNotar` once repair has the block and the parent has a
notar-fallback certificate or a stronger one. In the model-based tests,
`agave_pool` failed this way, for example with
`pool.safeToNotar[v3]: only in the spec: BlockRef { slot: 2, hash: 120 }`
(`QUINT_SEED=0xd8b94300`): v3 had skipped slot 2, block 120 had 5 of 9
notarize votes, and its parent 110 had 5 of 9 too, short of a certificate.
The patch adds, outside the first slot of a window,
`parentNotarFallbackCertified(b, ...)`: a `NotarFallbackCertificate` on the
parent (which a notarization or fast-finalization certificate implies, since
their votes count towards it), with the genesis parent (hash -1) counted as
certified, as `init` and patch 14 do. A parent that is not one of the
environment's blocks has no certificate, since no correct process votes for
it. It changes the condition for every instance. Together, patches 15 and 16
change 44, 39, 18, 46, and 47 of 50 traces of `some_byz`, `some_byz_vp`,
`agave_window`, `agave_gen`, and `agave_pool` (seed `0x1234`).

The paper is paraphrased here, not quoted: no copy of it was at hand when the
patch was written. Agave's `consensus_pool/slot_stake_counters.rs` quotes the
first part of Definition 16 (white paper v1.1, page 22), and
`consensus_pool_service.rs` implements the deferral. The citation is to be
checked against the paper before reporting the patch upstream.

The service also drops a waiting block once the block's slot is at or below
the highest finalized slot. The spec has no such cutoff, and this is not a
spec patch, because the cutoff is an optimization (the notar-fallback vote it
avoids is in a finalized slot). The model-based tests document it as a
trusted difference and stand in for agave there (see the `mbt` module doc).

### Patch 17: `safeToSkipCondition` requires that the process did not vote to skip

`statemachine.qnt` (`safeToSkipCondition`).

The paper (Definition 16) issues `SafeToSkip(s)` only "if the node voted in
slot s already, but not to skip s", and the comment in the spec says the
same. Upstream only requires some message from the process in slot `s` other
than `SkipVoteMsg(s)`, so the condition also holds for a process that voted
to skip `s` and then cast a notar-fallback vote in it. That process then casts
a skip-fallback vote next to its skip vote. Agave never does: its pool emits
`SafeToSkip` only if the node's first vote in the slot was a notarize vote.
Agave's pool also cannot build a skip certificate that counts both votes of
one node, since the skip and skip-fallback signers of a certificate must be
disjoint, so with the pool under test (patch 12) the spec had skip
certificates that agave's pools could not form. The patch requires that the
process sent some message for `s` and no `SkipVoteMsg(s)`. It changes the
condition for every instance.

### Patch 18: skip-fallback votes alone can skip-certify a slot for `ParentReady`

`alpenglow.qnt` (`slotsSkipCertified`).

`slotsSkipCertified(msgs)` only considers the slots that have a
`SkipVoteMsg`, and then keeps those for which `isCertified(SkipCertificate(s),
msgs)` holds. A skip certificate counts skip and skip-fallback votes
(Table 5, and `isCertified`), so a slot whose skip certificate is made of
skip-fallback votes alone was left out. That happens when the processes voted
to notarize different blocks of the slot and then received `SafeToSkip`. Such
a slot then blocked `parentReadyBlocks` for the next window, while agave's
pools, which build the skip certificate from either kind of vote, emitted the
`ParentReady`. After patch 14, two of three runs of `agave_pool` failed this
way, for example with `pool.parentReady: only in agave: (4, -1)`
(`QUINT_SEED=0xd47703c2`); `quint run --invariant` on a check that every
certified slot is in `slotsSkipCertified` found a trace where three
skip-fallback votes certify slot 0. The patch also collects the slots of
`SkipFallbackVoteMsg`. `parentReadyBlocks` is the only reader, so this
changes `parentReadyAction`, in every instance with a second window. The bug
is upstream too, and is to be reported upstream.

### Patch 19: witnesses of the targeted scenarios

`statemachine.qnt` (`parentReadySlot4AfterSkip`, `notarFallbackForOtherBlock`,
`finalizedAfterSkipVote`, `blockFastFinalized`,
`intraWindowNotarFallbackVoted`, `parentReadyGenesisSlot4`,
`skipCertifiedByFallbackOnly`).

Not a fix, and not to be reported upstream. The targets tests
(`mbt_votor_agave_gen_targets` and `mbt_votor_agave_pool_targets`) replay
only traces that reach a scenario: each target is `not(<witness>)`, passed to
`quint run --invariant`, so every trace that violates it reaches the scenario
and ends at the first state where it does. The patch adds one witness `val`
per scenario, next to patch 9's witnesses, two of which
(`notarizedInWindow1` and `skipFallbackVoted`) are targets too. See the
target catalogue below.

Like patch 9's witnesses, the vals are observations: no action reads them.
With and without the patch, `quint run --mbt` produces the same traces, apart
from the creation time in `#meta`, for 50 traces of each of the five
instances at seeds `0x1234` and `0x4321`.

### Not patched: `safeToSkipCondition` excludes the block with the most voters

`safeToSkipCondition` computes `skip(s) + Σ notar(b) − max notar(b)` over
sender sets, and picks the block to leave out by the number of its notarize
voters (`votedToNotar(x, ...).size()`), while the paper and agave leave out
the block with the most notarize stake. With unequal stakes the two can pick
different blocks. In `agave_pool` (stakes 3, 2, 2, and 2) this never changes
whether the 40% threshold is reached, so the model-based tests cannot see it
and the spec is left as it is (see the `mbt` module doc). It is to be reported
upstream together with the patches.

### Addition: the `agave_window` instance

`statemachine.qnt`.

Not upstream. The upstream instances only have slots 0..2, so slot 0 is their
only first slot of a leader window and `parentReadyAction` never fires. In
`agave_window`, slot 4 starts a second window, and blocks 50 and 51 in slot 4
have parents 42 and 43 in the first window. Slot 4 can become parent-ready
for 42, 43, or the Byzantine 46; all three occur in generated traces. v1 (stake 3) and b1 (stake 1)
together reach the 60% certificate threshold, so v1 notarizing 42 and timing
out a later slot of window 0 is enough to make slot 4 parent-ready.

## Target catalogue

The scenarios below are reached by only a share of random traces. Each was
measured first with `quint run --witnesses` over 200 traces of 60 steps
(`--init=initGenerated`, seeds `0x1234` and `0x4321`). A scenario reached by
at least about 1% of traces became a target, with `witnesses` sized so that
`quint run --invariant` takes about 10 s or less on its own. Two scenarios
reached by fewer traces (one of them only in `agave_pool`), and two that are
not a property of a single state, were left out.

`notarizedInWindow1` (Phase 10's target) keeps its 50 witnesses. It is also a
target in `agave_pool`, where 9 to 10% of traces reach it, which the candidate
list did not plan.

### Targets

Hit rate: share of 200 traces at seed `0x1234` / `0x4321`. Quint time: one
`quint run --invariant` on its own, at seed `0x1234`. In the tests, every
target's `quint` runs at the same time as the others (and as the simulation
tests), so the times the tests print are longer.

| Scenario | Instance | Witness | Hit rate | Witnesses (max samples) | Quint time |
|---|---|---|---|---|---|
| A block notarized in window 1 | `agave_gen` | `notarizedInWindow1` | 14.5% / 18% | 50 (1000) | 13.1 s |
| `ParentReady` for slot 4 with a skipped slot in window 0 | `agave_gen` | `parentReadySlot4AfterSkip` | 91% / 93.5% | 20 (100) | 1.8 s |
| A notar-fallback vote for a block other than the process's notarize vote | `agave_gen` | `notarFallbackForOtherBlock` | 14% / 11% | 12 (500) | 8.6 s |
| A skip-fallback vote (always after the process's notarize vote, by patch 17) | `agave_gen` | `skipFallbackVoted` | 19.5% / 18% | 25 (500) | 7.3 s |
| A slot finalized in a window where an earlier slot got a skip vote | `agave_gen` | `finalizedAfterSkipVote` | 8% / 8% | 10 (600) | 8.5 s |
| A block notarized in window 1 | `agave_pool` | `notarizedInWindow1` | 9% / 10% | 20 (600) | 9.5 s |
| `ParentReady` for slot 4 with a skipped slot in window 0 | `agave_pool` | `parentReadySlot4AfterSkip` | 73.5% / 75% | 20 (100) | 2.0 s |
| A notar-fallback vote for a block other than the process's notarize vote | `agave_pool` | `notarFallbackForOtherBlock` | 9% / 8% | 20 (600) | 8.3 s |
| A skip-fallback vote | `agave_pool` | `skipFallbackVoted` | 33% / 29.5% | 25 (300) | 3.3 s |
| Fast finalization | `agave_pool` | `blockFastFinalized` | 30% / 35.5% | 25 (300) | 3.9 s |
| An intra-window `SafeToNotar`, resolved through the pending path | `agave_pool` | `intraWindowNotarFallbackVoted` | 24.5% / 31.5% | 25 (300) | 5.0 s |
| `ParentReady` for genesis at slot 4 (patch 14) | `agave_pool` | `parentReadyGenesisSlot4` | 22.5% / 19.5% | 25 (300) | 4.2 s |

Notes:

- "A slot finalized in a window where an earlier slot was skipped" is read
  across processes: some process voted to skip an earlier slot of the
  finalized slot's window. A single process cannot do both, because its
  notarize votes in a window form a prefix that starts at the window's first
  slot, and a skip vote skips every slot of the window it has not voted in.
  The single-process reading (a process voted to finalize a slot, and has
  `BadWindow` on an earlier slot of the window, from a fallback vote) was
  reached in 0 of 800 traces (both instances, both seeds), and the reading
  over skip certificates in 0 of 400 traces of `agave_gen`.
- "Fast finalization" was planned as `FinalizeFast` without `Notarize`
  (Phase 7's deviation 1). With `agave_pool`'s stakes, no single vote can
  form `FinalizeFast` first (patch 12), so the target is any fast
  finalization. The coverage below shows `certificate FinalizeFast` and
  `certificate Notarize` in every witness.
- Every intra-window `SafeToNotar` in agave goes through the consensus pool
  service's pending path, so "resolved through the pending path" holds for
  every witness of `intraWindowNotarFallbackVoted`. The target stops at the
  notar-fallback vote, so the event was also dispatched.

### Left out

| Scenario | Instance | Hit rate | Why it is left out |
|---|---|---|---|
| Two blocks pending in one slot, and the earlier one is voted once its parent is (patch 1's order) | both | not a witness; measured offline: 0.5% / 1% (`agave_gen`), 2.5% / 2% (`agave_pool`) | Not a property of a single state. When a process votes, `tryNotar` clears the slot's pending blocks, so the state no longer shows that the voted block was pending, nor in which order. A witness of the precondition alone (two blocks pending in one slot, 95% to 99.5% of traces) ends before the vote. Needs a recorded history in the spec, or a search backend. The offline rate counts traces in which a process votes, out of its pending blocks, for the earlier of two pending blocks that have the same parent, computed from the ITF states of 200 traces. Agave's pending blocks are compared in arrival order after every step, so the simulation tests still check this order where it occurs. |
| A block voted out of `pendingBlocks` after its parent's slot was voted (the pending path of `try_notar`) | both | not a witness; measured offline: 37% / 30.5% (`agave_gen`), 38% / 39.5% (`agave_pool`) | Not a property of a single state, for the same reason. The simulation tests reach it in about a third of their traces. |
| A slot finalized in a window where an earlier slot got a skip vote | `agave_pool` | 0.5% / 1% | Too rare: needs a biased `init`/`step` or a search backend. Kept for `agave_gen`. |
| A skip certificate made of skip-fallback votes only (patch 18) | `agave_pool` | 0.5% / 0% (`skipCertifiedByFallbackOnly`) | Too rare: needs a biased `init`/`step` or a search backend. The stronger scenario, where such a slot makes a window start parent-ready (the one patch 18 changes), was reached in 1 and 0 of 200 traces. In `agave_gen`, 0 / 0. |

### Coverage

What each target's witnesses drove agave through, as the targets tests print
it (`QUINT_SEED=0x1234 cargo nextest run -p agave-votor --features
agave-unstable-api --run-ignored ignored-only --no-capture targets`). Every
witness of a target reaches its scenario, and the counter for the agave path
the target is named after is non-zero in each block: `blockNotarizedAction`
for window 1, `parentReadyAction`, `NotarFallBackVoteMsg` and
`safeToNotarAction`, `SkipFallbackVoteMsg` and `safeToSkipAction`,
`FinalVoteMsg`, `certificate FinalizeFast`, `pending SafeToNotar resolved`
and `intra-window SafeToNotar dispatched`, and `dispatched ParentReady`.

```text
votor MBT target 'notarized in window 1' (not(notarizedInWindow1)) of agave_gen: 50/50 witnesses in
    at most 1000 samples, quint 15.7 s, replay 1.3 s, states per witness min/median/max 14/35/60
  traces 50, actions {"blockNotarizedAction": 141, "fireTimeoutEvent": 311, "initGenerated": 50,
      "parentReadyAction": 192, "receiveBlock": 1069, "safeToNotarAction": 38, "safeToSkipAction":
      10}, votes pushed 1076
  traces whose block tree has: Equivocation 44 (88%), PhantomParent 16 (32%), FarParent 46 (92%),
      LateGenesisParent 35 (70%), EmptySlot 38 (76%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 38 (76%),
      NotarFallBackVoteMsg 19 (38%), NotarVoteMsg 50 (100%), SkipFallbackVoteMsg 8 (16%),
      SkipVoteMsg 50 (100%), blockNotarizedAction 50 (100%), fireTimeoutEvent 50 (100%),
      initGenerated 50 (100%), parentReadyAction 50 (100%), receiveBlock 50 (100%),
      safeToNotarAction 19 (38%), safeToSkipAction 8 (16%)

votor MBT target 'ParentReady for slot 4 after a skipped slot' (not(parentReadySlot4AfterSkip)) of
    agave_gen: 20/20 witnesses in at most 100 samples, quint 2.9 s, replay 0.3 s, states per witness
    min/median/max 6/23/53
  traces 20, actions {"blockNotarizedAction": 29, "fireTimeoutEvent": 58, "initGenerated": 20,
      "parentReadyAction": 22, "receiveBlock": 375, "safeToNotarAction": 3}, votes pushed 227
  traces whose block tree has: Equivocation 17 (85%), PhantomParent 13 (65%), FarParent 20 (100%),
      LateGenesisParent 13 (65%), EmptySlot 12 (60%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 10 (50%),
      NotarFallBackVoteMsg 3 (15%), NotarVoteMsg 17 (85%), SkipVoteMsg 20 (100%),
      blockNotarizedAction 11 (55%), fireTimeoutEvent 20 (100%), initGenerated 20 (100%),
      parentReadyAction 20 (100%), receiveBlock 20 (100%), safeToNotarAction 3 (15%)

votor MBT target 'notar-fallback vote for a block other than the own notarize vote'
    (not(notarFallbackForOtherBlock)) of agave_gen: 12/12 witnesses in at most 500 samples, quint
    10.1 s, replay 0.2 s, states per witness min/median/max 7/36/52
  traces 12, actions {"blockNotarizedAction": 21, "fireTimeoutEvent": 42, "initGenerated": 12,
      "parentReadyAction": 36, "receiveBlock": 237, "safeToNotarAction": 17, "safeToSkipAction": 2},
      votes pushed 191
  traces whose block tree has: Equivocation 12 (100%), PhantomParent 7 (58%), FarParent 10 (83%),
      LateGenesisParent 9 (75%), EmptySlot 5 (41%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 6 (50%),
      NotarFallBackVoteMsg 12 (100%), NotarVoteMsg 12 (100%), SkipFallbackVoteMsg 2 (16%),
      SkipVoteMsg 12 (100%), blockNotarizedAction 8 (66%), fireTimeoutEvent 11 (91%), initGenerated
      12 (100%), parentReadyAction 9 (75%), receiveBlock 12 (100%), safeToNotarAction 12 (100%),
      safeToSkipAction 2 (16%)

votor MBT target 'skip-fallback vote' (not(skipFallbackVoted)) of agave_gen: 25/25 witnesses in at
    most 500 samples, quint 9.1 s, replay 0.5 s, states per witness min/median/max 13/37/60
  traces 25, actions {"blockNotarizedAction": 30, "fireTimeoutEvent": 130, "initGenerated": 25,
      "parentReadyAction": 143, "receiveBlock": 577, "safeToNotarAction": 43, "safeToSkipAction":
      25}, votes pushed 484
  traces whose block tree has: Equivocation 22 (88%), PhantomParent 10 (40%), FarParent 22 (88%),
      LateGenesisParent 17 (68%), EmptySlot 12 (48%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 11 (44%),
      NotarFallBackVoteMsg 14 (56%), NotarVoteMsg 25 (100%), SkipFallbackVoteMsg 25 (100%),
      SkipVoteMsg 25 (100%), blockNotarizedAction 13 (52%), fireTimeoutEvent 25 (100%),
      initGenerated 25 (100%), parentReadyAction 23 (92%), receiveBlock 25 (100%), safeToNotarAction
      14 (56%), safeToSkipAction 25 (100%)

votor MBT target 'slot finalized after a skip vote earlier in its window'
    (not(finalizedAfterSkipVote)) of agave_gen: 10/10 witnesses in at most 600 samples, quint 9.7 s,
    replay 0.2 s, states per witness min/median/max 22/54/61
  traces 10, actions {"blockNotarizedAction": 63, "fireTimeoutEvent": 43, "initGenerated": 10,
      "parentReadyAction": 57, "receiveBlock": 289, "safeToNotarAction": 12, "safeToSkipAction": 1},
      votes pushed 204
  traces whose block tree has: Equivocation 8 (80%), PhantomParent 3 (30%), FarParent 10 (100%),
      LateGenesisParent 5 (50%), EmptySlot 8 (80%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 10 (100%),
      NotarFallBackVoteMsg 6 (60%), NotarVoteMsg 10 (100%), SkipFallbackVoteMsg 1 (10%), SkipVoteMsg
      10 (100%), blockNotarizedAction 10 (100%), fireTimeoutEvent 10 (100%), initGenerated 10
      (100%), parentReadyAction 9 (90%), receiveBlock 10 (100%), safeToNotarAction 6 (60%),
      safeToSkipAction 1 (10%)

votor MBT target 'notarized in window 1' (not(notarizedInWindow1)) of agave_pool: 20/20 witnesses in
    at most 600 samples, quint 12.9 s, replay 0.8 s, states per witness min/median/max 28/45/61
  traces 20, actions {"blockNotarizedAction": 44, "fireTimeoutEvent": 133, "initGenerated": 20,
      "parentReadyAction": 84, "receiveBlock": 578, "safeToNotarAction": 14, "safeToSkipAction":
      16}, votes pushed 546
  traces whose block tree has: Equivocation 15 (75%), PhantomParent 8 (40%), FarParent 17 (85%),
      LateGenesisParent 14 (70%), EmptySlot 15 (75%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 14 (70%),
      NotarFallBackVoteMsg 11 (55%), NotarVoteMsg 20 (100%), SkipFallbackVoteMsg 8 (40%),
      SkipVoteMsg 20 (100%), blockNotarizedAction 20 (100%), certificate Finalize 3 (15%),
      certificate FinalizeFast 8 (40%), certificate Notarize 20 (100%), certificate NotarizeFallback
      9 (45%), certificate Skip 20 (100%), dispatched BlockNotarized 20 (100%), dispatched
      ParentReady 20 (100%), dispatched SafeToNotar 11 (55%), dispatched SafeToNotar (intra-window)
      2 (10%), dispatched SafeToSkip 8 (40%), fireTimeoutEvent 20 (100%), initGenerated 20 (100%),
      parentReadyAction 20 (100%), pending SafeToNotar resolved 8 (40%), receiveBlock 20 (100%),
      safeToNotarAction 11 (55%), safeToSkipAction 8 (40%)
  pool events emitted/dispatched/asked for again: BlockNotarFallback 36/0/0, BlockNotarized
      152/36/8, Finalized(fast) 36/0/0, Finalized(slow) 12/0/0, ParentReady 92/73/11, SafeToNotar
      47/11/3, SafeToSkip 40/14/2
  pending SafeToNotar: resolved 21, dropped past the highest finalized slot 0, stood in for by the
      driver 0, intra-window SafeToNotar dispatched 2

votor MBT target 'ParentReady for slot 4 after a skipped slot' (not(parentReadySlot4AfterSkip)) of
    agave_pool: 20/20 witnesses in at most 100 samples, quint 4.3 s, replay 0.5 s, states per
    witness min/median/max 8/29/59
  traces 20, actions {"blockNotarizedAction": 27, "fireTimeoutEvent": 77, "initGenerated": 20,
      "parentReadyAction": 20, "receiveBlock": 453, "safeToNotarAction": 7, "safeToSkipAction": 9},
      votes pushed 329
  traces whose block tree has: Equivocation 18 (90%), PhantomParent 10 (50%), FarParent 18 (90%),
      LateGenesisParent 15 (75%), EmptySlot 12 (60%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 9 (45%),
      NotarFallBackVoteMsg 4 (20%), NotarVoteMsg 18 (90%), SkipFallbackVoteMsg 6 (30%), SkipVoteMsg
      20 (100%), blockNotarizedAction 9 (45%), certificate Finalize 1 (5%), certificate FinalizeFast
      5 (25%), certificate Notarize 12 (60%), certificate NotarizeFallback 3 (15%), certificate Skip
      20 (100%), dispatched BlockNotarized 9 (45%), dispatched ParentReady 20 (100%), dispatched
      SafeToNotar 4 (20%), dispatched SafeToNotar (intra-window) 2 (10%), dispatched SafeToSkip 6
      (30%), fireTimeoutEvent 20 (100%), initGenerated 20 (100%), parentReadyAction 20 (100%),
      pending SafeToNotar resolved 6 (30%), receiveBlock 20 (100%), safeToNotarAction 4 (20%),
      safeToSkipAction 6 (30%)
  pool events emitted/dispatched/asked for again: BlockNotarFallback 16/0/0, BlockNotarized 68/21/6,
      Finalized(fast) 24/0/0, Finalized(slow) 4/0/0, ParentReady 80/20/0, SafeToNotar 39/7/0,
      SafeToSkip 29/9/0
  pending SafeToNotar: resolved 17, dropped past the highest finalized slot 0, stood in for by the
      driver 0, intra-window SafeToNotar dispatched 4

votor MBT target 'notar-fallback vote for a block other than the own notarize vote'
    (not(notarFallbackForOtherBlock)) of agave_pool: 20/20 witnesses in at most 600 samples, quint
    11.6 s, replay 0.5 s, states per witness min/median/max 8/41/60
  traces 20, actions {"blockNotarizedAction": 27, "fireTimeoutEvent": 73, "initGenerated": 20,
      "parentReadyAction": 22, "receiveBlock": 556, "safeToNotarAction": 27, "safeToSkipAction": 8},
      votes pushed 351
  traces whose block tree has: Equivocation 20 (100%), PhantomParent 9 (45%), FarParent 19 (95%),
      LateGenesisParent 14 (70%), EmptySlot 12 (60%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 7 (35%),
      NotarFallBackVoteMsg 20 (100%), NotarVoteMsg 20 (100%), SkipFallbackVoteMsg 7 (35%),
      SkipVoteMsg 20 (100%), blockNotarizedAction 8 (40%), certificate Finalize 2 (10%), certificate
      FinalizeFast 2 (10%), certificate Notarize 10 (50%), certificate NotarizeFallback 16 (80%),
      certificate Skip 12 (60%), dispatched BlockNotarized 8 (40%), dispatched ParentReady 4 (20%),
      dispatched SafeToNotar 20 (100%), dispatched SafeToNotar (intra-window) 6 (30%), dispatched
      SafeToSkip 7 (35%), fireTimeoutEvent 18 (90%), initGenerated 20 (100%), parentReadyAction 4
      (20%), pending SafeToNotar resolved 8 (40%), receiveBlock 20 (100%), safeToNotarAction 20
      (100%), safeToSkipAction 7 (35%)
  pool events emitted/dispatched/asked for again: BlockNotarFallback 68/0/0, BlockNotarized 76/26/1,
      Finalized(fast) 12/0/0, Finalized(slow) 8/0/0, ParentReady 60/13/9, SafeToNotar 72/25/2,
      SafeToSkip 59/8/0
  pending SafeToNotar: resolved 32, dropped past the highest finalized slot 0, stood in for by the
      driver 0, intra-window SafeToNotar dispatched 9

votor MBT target 'skip-fallback vote' (not(skipFallbackVoted)) of agave_pool: 25/25 witnesses in at
    most 300 samples, quint 6.5 s, replay 0.6 s, states per witness min/median/max 10/34/59
  traces 25, actions {"blockNotarizedAction": 32, "fireTimeoutEvent": 83, "initGenerated": 25,
      "parentReadyAction": 12, "receiveBlock": 672, "safeToNotarAction": 21, "safeToSkipAction":
      25}, votes pushed 458
  traces whose block tree has: Equivocation 24 (96%), PhantomParent 13 (52%), FarParent 23 (92%),
      LateGenesisParent 15 (60%), EmptySlot 9 (36%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 8 (32%),
      NotarFallBackVoteMsg 10 (40%), NotarVoteMsg 25 (100%), SkipFallbackVoteMsg 25 (100%),
      SkipVoteMsg 25 (100%), blockNotarizedAction 8 (32%), certificate Finalize 3 (12%), certificate
      FinalizeFast 3 (12%), certificate Notarize 12 (48%), certificate NotarizeFallback 9 (36%),
      certificate Skip 21 (84%), dispatched BlockNotarized 8 (32%), dispatched ParentReady 4 (16%),
      dispatched SafeToNotar 10 (40%), dispatched SafeToNotar (intra-window) 4 (16%), dispatched
      SafeToSkip 25 (100%), fireTimeoutEvent 22 (88%), initGenerated 25 (100%), parentReadyAction 4
      (16%), pending SafeToNotar resolved 15 (60%), receiveBlock 25 (100%), safeToNotarAction 10
      (40%), safeToSkipAction 25 (100%)
  pool events emitted/dispatched/asked for again: BlockNotarFallback 48/0/0, BlockNotarized
      56/22/10, Finalized(fast) 16/0/0, Finalized(slow) 12/0/0, ParentReady 52/10/2, SafeToNotar
      93/16/5, SafeToSkip 106/25/0
  pending SafeToNotar: resolved 42, dropped past the highest finalized slot 0, stood in for by the
      driver 0, intra-window SafeToNotar dispatched 7

votor MBT target 'fast finalization' (not(blockFastFinalized)) of agave_pool: 25/25 witnesses in at
    most 300 samples, quint 6.8 s, replay 0.4 s, states per witness min/median/max 7/18/51
  traces 25, actions {"blockNotarizedAction": 27, "fireTimeoutEvent": 35, "initGenerated": 25,
      "parentReadyAction": 10, "receiveBlock": 419, "safeToNotarAction": 4, "safeToSkipAction": 5},
      votes pushed 246
  traces whose block tree has: Equivocation 18 (72%), PhantomParent 10 (40%), FarParent 23 (92%),
      LateGenesisParent 18 (72%), EmptySlot 13 (52%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 13 (52%),
      NotarFallBackVoteMsg 2 (8%), NotarVoteMsg 25 (100%), SkipFallbackVoteMsg 2 (8%), SkipVoteMsg
      14 (56%), blockNotarizedAction 13 (52%), certificate FinalizeFast 25 (100%), certificate
      Notarize 25 (100%), certificate NotarizeFallback 2 (8%), certificate Skip 2 (8%), dispatched
      BlockNotarized 13 (52%), dispatched ParentReady 2 (8%), dispatched SafeToNotar 2 (8%),
      dispatched SafeToNotar (intra-window) 1 (4%), dispatched SafeToSkip 2 (8%), fireTimeoutEvent
      15 (60%), initGenerated 25 (100%), parentReadyAction 2 (8%), pending SafeToNotar dropped 1
      (4%), pending SafeToNotar resolved 5 (20%), receiveBlock 25 (100%), safeToNotarAction 2 (8%),
      safeToSkipAction 2 (8%)
  pool events emitted/dispatched/asked for again: BlockNotarFallback 8/0/0, BlockNotarized 120/21/6,
      Finalized(fast) 100/0/0, ParentReady 12/9/1, SafeToNotar 11/3/1, SafeToSkip 11/4/1
  pending SafeToNotar: resolved 8, dropped past the highest finalized slot 6, stood in for by the
      driver 0, intra-window SafeToNotar dispatched 2

votor MBT target 'intra-window SafeToNotar' (not(intraWindowNotarFallbackVoted)) of agave_pool:
    25/25 witnesses in at most 300 samples, quint 8.3 s, replay 0.6 s, states per witness
    min/median/max 22/37/60
  traces 25, actions {"blockNotarizedAction": 70, "fireTimeoutEvent": 107, "initGenerated": 25,
      "parentReadyAction": 39, "receiveBlock": 679, "safeToNotarAction": 30, "safeToSkipAction": 9},
      votes pushed 486
  traces whose block tree has: Equivocation 23 (92%), PhantomParent 12 (48%), FarParent 21 (84%),
      LateGenesisParent 17 (68%), EmptySlot 12 (48%)
  traces with each action, vote, dispatched pool event, or pool certificate: FinalVoteMsg 20 (80%),
      NotarFallBackVoteMsg 25 (100%), NotarVoteMsg 25 (100%), SkipFallbackVoteMsg 7 (28%),
      SkipVoteMsg 25 (100%), blockNotarizedAction 21 (84%), certificate Finalize 3 (12%),
      certificate FinalizeFast 9 (36%), certificate Notarize 23 (92%), certificate NotarizeFallback
      14 (56%), certificate Skip 14 (56%), dispatched BlockNotarized 21 (84%), dispatched
      ParentReady 6 (24%), dispatched SafeToNotar 25 (100%), dispatched SafeToNotar (intra-window)
      25 (100%), dispatched SafeToSkip 7 (28%), fireTimeoutEvent 25 (100%), initGenerated 25 (100%),
      parentReadyAction 6 (24%), pending SafeToNotar resolved 25 (100%), receiveBlock 25 (100%),
      safeToNotarAction 25 (100%), safeToSkipAction 7 (28%)
  pool events emitted/dispatched/asked for again: BlockNotarFallback 64/0/0, BlockNotarized
      148/51/19, Finalized(fast) 44/0/0, Finalized(slow) 12/0/0, ParentReady 80/21/18, SafeToNotar
      90/29/1, SafeToSkip 54/9/0
  pending SafeToNotar: resolved 66, dropped past the highest finalized slot 0, stood in for by the
      driver 0, intra-window SafeToNotar dispatched 25

votor MBT target 'ParentReady for genesis at slot 4' (not(parentReadyGenesisSlot4)) of agave_pool:
    25/25 witnesses in at most 300 samples, quint 7.2 s, replay 0.6 s, states per witness
    min/median/max 5/25/61
  traces 25, actions {"fireTimeoutEvent": 108, "initGenerated": 25, "parentReadyAction": 65,
      "receiveBlock": 414, "safeToNotarAction": 18, "safeToSkipAction": 36}, votes pushed 456
  traces whose block tree has: Equivocation 23 (92%), PhantomParent 16 (64%), FarParent 22 (88%),
      LateGenesisParent 19 (76%), EmptySlot 21 (84%)
  traces with each action, vote, dispatched pool event, or pool certificate: NotarFallBackVoteMsg 9
      (36%), NotarVoteMsg 18 (72%), SkipFallbackVoteMsg 14 (56%), SkipVoteMsg 25 (100%), certificate
      NotarizeFallback 7 (28%), certificate Skip 25 (100%), dispatched ParentReady 25 (100%),
      dispatched SafeToNotar 9 (36%), dispatched SafeToNotar (intra-window) 3 (12%), dispatched
      SafeToSkip 14 (56%), fireTimeoutEvent 25 (100%), initGenerated 25 (100%), parentReadyAction 25
      (100%), pending SafeToNotar resolved 4 (16%), receiveBlock 23 (92%), safeToNotarAction 9
      (36%), safeToSkipAction 14 (56%)
  pool events emitted/dispatched/asked for again: BlockNotarFallback 36/0/0, ParentReady 136/47/18,
      SafeToNotar 56/14/4, SafeToSkip 56/28/8
  pending SafeToNotar: resolved 10, dropped past the highest finalized slot 0, stood in for by the
      driver 0, intra-window SafeToNotar dispatched 3
```
