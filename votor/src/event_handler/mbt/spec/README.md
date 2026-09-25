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
and 4 are to be reported upstream.

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

### Addition: the `agave_window` instance

`statemachine.qnt`.

Not upstream. The upstream instances only have slots 0..2, so slot 0 is their
only first slot of a leader window and `parentReadyAction` never fires. In
`agave_window`, slot 4 starts a second window, and blocks 50 and 51 in slot 4
have parents 42 and 43 in the first window. Slot 4 can become parent-ready
for 42, 43, or the Byzantine 46; all three occur in generated traces. v1 (stake 3) and b1 (stake 1)
together reach the 60% certificate threshold, so v1 notarizing 42 and timing
out a later slot of window 0 is enough to make slot 4 parent-ready.
