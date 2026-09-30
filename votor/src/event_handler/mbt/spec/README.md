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
4, 5, 6, 11, 14, 17, and 18 are to be reported upstream. Numbers 15 and 16
are reserved for later patches that compare agave's consensus pool with the
spec; patches 17 and 18 were needed first.

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
`initWith`, `fireTimeoutEvent`, `processInput`).

Not a fix, and not to be reported upstream. The model-based tests compare
agave's consensus pools with the spec, but ITF traces only record state
variables, and the spec's certificates are conditions over the message soup.
The patch adds a `pool` field to `Environment` that `initWith`,
`fireTimeoutEvent`, and `processInput` recompute after the step's effect.
`poolView` holds every notarization, notar-fallback, and fast-finalization
certificate on one of the trace's blocks, and every skip and finalization
certificate on an alive slot, for which `isCertified` holds over
`msgBuffer` and the Byzantine soup, and every pair `(s, hash(b))` with `b` in
`parentReadyBlocks(s, ...)`, plus the `(0, -1)` that `init` preloads. Each
candidate certificate is checked against the messages of its own slot only,
which gives the same result faster.

No action reads the field, so no trace changes apart from it: with and without
the patch, `quint run --mbt` produces the same actions, picks, and states
(ignoring `pool`) for 50 traces of each of the five instances, at seeds
`0x1234` and `0x4321`. The view is only computed in instances without
Byzantine processes, the ones where the model-based tests run agave's pools
(patch 12). Elsewhere it stays empty, because evaluating it over the Byzantine
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

### Addition: the `agave_window` instance

`statemachine.qnt`.

Not upstream. The upstream instances only have slots 0..2, so slot 0 is their
only first slot of a leader window and `parentReadyAction` never fires. In
`agave_window`, slot 4 starts a second window, and blocks 50 and 51 in slot 4
have parents 42 and 43 in the first window. Slot 4 can become parent-ready
for 42, 43, or the Byzantine 46; all three occur in generated traces. v1 (stake 3) and b1 (stake 1)
together reach the 60% certificate threshold, so v1 notarizing 42 and timing
out a later slot of window 0 is enough to make slot 4 parent-ready.
