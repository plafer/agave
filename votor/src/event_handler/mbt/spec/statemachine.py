"""Generated from Quint by `wunderspec convert`."""

from __future__ import annotations

from typing import Callable, cast

from wunderspec import (
    AllMaps,
    AllTuples,
    And,
    BoolExpr,
    Context,
    Expr,
    Field,
    IntExpr,
    Ite,
    List,
    Map,
    Or,
    Param,
    Range,
    Record,
    Set,
    StateVar,
    Tuple,
    UnionExpr,
    Unit,
    Val,
    Variant,
    action,
    instance,
    record,
    state,
    union,
)
from wunderspec.machine import MachineStateBase


def _quint_sort_list(list_expr, lt):
    elem_sort = list_expr.sort.elem_sort

    def insert_in_order(sorted_list, item):
        insertion = sorted_list.reduce(
            lambda acc, current: Ite(
                acc[1],
                Tuple(acc[0] + List(current), Val(True)),
                Ite(
                    lt(current, item),
                    Tuple(acc[0] + List(current), Val(False)),
                    Tuple(acc[0] + List(item) + List(current), Val(True)),
                ),
            ),
            Tuple(List(elem_sort), Val(False)),
        )
        return Ite(insertion[1], insertion[0], (insertion[0] + List(item)))

    return list_expr.reduce(
      (lambda sorted_list, item: (insert_in_order(sorted_list, item))),
      List(elem_sort)
    )


@record
class Block:
    slot: Field[int]
    hash: Field[int]
    parent: Field[int]

@record
class BlockReference:
    slot: Field[int]
    hash: Field[int]

@union
class SlotObject:
    ParentReady: Variant[int]
    Voted: Variant[Unit]
    VotedNotar: Variant[int]
    BlockNotarized: Variant[int]
    ItsOver: Variant[Unit]
    BadWindow: Variant[Unit]
    VotedNotarFallback: Variant[int]
    VotedSkipFallback: Variant[Unit]

@record
class LocalState:
    pendingBlocks: Field[list[list[Block]]]
    state: Field[list[set[SlotObject]]]

@union
class ConsensusInput:
    BlockInput: Variant[Block]
    TimeOutInput: Variant[int]
    BlockNotarizedInput: Variant[BlockReference]
    ParentReadyInput: Variant[BlockReference]
    SafeToNotarInput: Variant[BlockReference]
    SafeToSkipInput: Variant[int]

@union
class Message:
    NotarVoteMsg: Variant[BlockReference]
    NotarFallBackVoteMsg: Variant[BlockReference]
    SkipVoteMsg: Variant[int]
    SkipFallbackVoteMsg: Variant[int]
    FinalVoteMsg: Variant[int]

@union
class Certificate:
    FastFinalizationCertificate: Variant[BlockReference]
    NotarizationCertificate: Variant[BlockReference]
    NotarFallbackCertificate: Variant[BlockReference]
    SkipCertificate: Variant[int]
    FinalizationCertificate: Variant[int]

@union
class ConsensusOutput:
    ScheduleEventTimeout: Variant[int]
    Broadcast: Variant[Message]

@record
class NetworkMsg:
    sender: Field[str]
    msg: Field[Message]

@record
class Result:
    output: Field[set[ConsensusOutput]]
    post: Field[LocalState]

@record
class PoolView:
    certificates: Field[set[Certificate]]
    parentReady: Field[set[tuple[int, int]]]
    safeToNotar: Field[dict[str, set[BlockReference]]]
    safeToSkip: Field[dict[str, set[int]]]

@record
class Environment:
    blocks: Field[set[Block]]
    system: Field[dict[str, LocalState]]
    msgBuffer: Field[set[NetworkMsg]]
    activeTimeouts: Field[dict[str, set[int]]]
    pool: Field[PoolView]

@union
class _QuintUnion1:
    NotarVoteMsg: Variant[BlockReference]
    NotarFallBackVoteMsg: Variant[BlockReference]
    SkipVoteMsg: Variant[int]
    SkipFallbackVoteMsg: Variant[int]
    FinalVoteMsg: Variant[int]

@union
class _QuintUnion2:
    FastFinalizationCertificate: Variant[BlockReference]
    NotarizationCertificate: Variant[BlockReference]
    NotarFallbackCertificate: Variant[BlockReference]
    SkipCertificate: Variant[int]
    FinalizationCertificate: Variant[int]

@union
class _QuintUnion3:
    ScheduleEventTimeout: Variant[int]
    Broadcast: Variant[_QuintUnion1]

@union
class _QuintUnion4:
    BlockInput: Variant[Block]
    TimeOutInput: Variant[int]
    BlockNotarizedInput: Variant[BlockReference]
    ParentReadyInput: Variant[BlockReference]
    SafeToNotarInput: Variant[BlockReference]
    SafeToSkipInput: Variant[int]

@union
class _QuintUnion5:
    None_: Variant[Unit]
    Some: Variant[int]

ProcessID = str

Slot = int

Blockhash = int

Bookkeeping = int

TimeoutEventData = int

@state
class AgavePoolState(MachineStateBase):
    correct: Param[set[str]]
    good: Param[set[str]]
    byzantine: Param[set[str]]
    power: Param[dict[str, int]]
    correctBlocks: Param[set[Block]]
    byzantineBlocks: Param[set[Block]]
    aliveSlots: Param[set[int]]
    aliveHashes: Param[set[int]]
    counter: StateVar[int]
    ch: StateVar[dict[int, set[int]]]
    s: StateVar[Environment]


@instance
def agave_pool() -> AgavePoolState:
    return AgavePoolState(
        correct=Set("v1", "v2", "v3", "v4"),
        good=Set(str),
        byzantine=Set(str),
        power=Map(("v1", 3), ("v2", 2), ("v3", 2), ("v4", 2)),
        correctBlocks=Set(Block),
        byzantineBlocks=Set(Block),
        aliveSlots=Set(0, ..., 7),
        aliveHashes=AllTuples(Set(0, ..., 7), Set(0, 1, 9)).map(
            (lambda t: ((Val(100) + (Val(10) * t[0])) + t[1]))
        ),
    )


def max(_state: AgavePoolState, i: Expr, j: Expr) -> Expr:
    return Ite((i > j), i, j)

def min(_state: AgavePoolState, i: Expr, j: Expr) -> Expr:
    return Ite((i < j), i, j)

def setAdd(_state: AgavePoolState, s: Expr, elem: Expr) -> Expr:
    return (s | Set(elem))

def listMap(_state: AgavePoolState, l: Expr, f: Callable[[Expr], Expr]) -> Expr:
    # `listMap` is generic in Quint: the result's element sort is the sort of `f`'s output.
    return Range(Val(0), l.size).reduce(
      (lambda acc, i: ((acc + List(f(l[i]))))),
      List(f(l[Val(0)]).sort)
    )

def firstSlotInLeaderWindow(_state: AgavePoolState, slot: Expr) -> BoolExpr:
    return ((slot % Val(4)) == Val(0))

def windowSlots(_state: AgavePoolState, slot: Expr) -> Expr:
    return Ite(
      ((slot % Val(4)) == Val(0)),
      Set(slot, (slot + Val(1)), (slot + Val(2)), (slot + Val(3))),
      Ite(
        ((slot % Val(4)) == Val(1)),
        Set((slot - Val(1)), slot, (slot + Val(1)), (slot + Val(2))),
        Ite(
          ((slot % Val(4)) == Val(2)),
          Set((slot - Val(2)), (slot - Val(1)), slot, (slot + Val(1))),
          Set((slot - Val(3)), (slot - Val(2)), (slot - Val(1)), slot)
        )
      )
    )

def ParentReady(_state: AgavePoolState, param: Expr) -> Expr:
    return SlotObject.ParentReady(param)

def Voted(_state: AgavePoolState) -> Expr:
    return SlotObject.Voted()

def VotedNotar(_state: AgavePoolState, param: Expr) -> Expr:
    return SlotObject.VotedNotar(param)

def BlockNotarized(_state: AgavePoolState, param: Expr) -> Expr:
    return SlotObject.BlockNotarized(param)

def ItsOver(_state: AgavePoolState) -> Expr:
    return SlotObject.ItsOver()

def BadWindow(_state: AgavePoolState) -> Expr:
    return SlotObject.BadWindow()

def VotedNotarFallback(_state: AgavePoolState, param: Expr) -> Expr:
    return SlotObject.VotedNotarFallback(param)

def VotedSkipFallback(_state: AgavePoolState) -> Expr:
    return SlotObject.VotedSkipFallback()

def benevolent(_state: AgavePoolState) -> Expr:
    return (_state.correct | _state.good)

def voting_power(_state: AgavePoolState, s: Expr) -> Expr:
    return s.reduce((lambda s, x: ((s + _state.power[x]))), Val(0))

def reference(_state: AgavePoolState, b: Expr) -> Expr:
    return Record(slot=b.slot, hash=b.hash)

def BlockInput(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion4.BlockInput(param)

def TimeOutInput(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion4.TimeOutInput(param)

def BlockNotarizedInput(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion4.BlockNotarizedInput(param)

def ParentReadyInput(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion4.ParentReadyInput(param)

def SafeToNotarInput(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion4.SafeToNotarInput(param)

def SafeToSkipInput(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion4.SafeToSkipInput(param)

def NotarVoteMsg(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion1.NotarVoteMsg(param)

def NotarFallBackVoteMsg(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion1.NotarFallBackVoteMsg(param)

def SkipVoteMsg(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion1.SkipVoteMsg(param)

def SkipFallbackVoteMsg(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion1.SkipFallbackVoteMsg(param)

def FinalVoteMsg(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion1.FinalVoteMsg(param)

def FastFinalizationCertificate(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion2.FastFinalizationCertificate(
      param
    )

def NotarizationCertificate(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion2.NotarizationCertificate(param)

def NotarFallbackCertificate(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion2.NotarFallbackCertificate(
      param
    )

def SkipCertificate(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion2.SkipCertificate(param)

def FinalizationCertificate(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion2.FinalizationCertificate(param)

def allBlocks(_state: AgavePoolState) -> Expr:
    return (_state.correctBlocks | _state.byzantineBlocks)

def totalVotingPower(_state: AgavePoolState) -> Expr:
    return voting_power(_state, ((_state.correct | _state.good) | _state.byzantine))

def slotOf(_state: AgavePoolState, msg: Expr) -> Expr:
    return UnionExpr(msg.node).match(
      NotarVoteMsg=(lambda m: (m.slot)),
      NotarFallBackVoteMsg=(lambda m: (m.slot)),
      SkipVoteMsg=(lambda slot: (slot)),
      SkipFallbackVoteMsg=(lambda slot: (slot)),
      FinalVoteMsg=(lambda slot: (slot))
    )

def addObjects(_state: AgavePoolState, ls: Expr, slot: Expr, obj: Expr) -> Expr:
    return ls.replace(state=ls.state.replace(slot, (ls.state[slot] | obj)))

def appendPendingBlock(_state: AgavePoolState, ls: Expr, slot: Expr, b: Expr) -> Expr:
    return ls.replace(
      pendingBlocks=ls.pendingBlocks.replace(
        slot,
        (ls.pendingBlocks[slot] + List(b))
      )
    )

def clearPendingBlocks(_state: AgavePoolState, ls: Expr, slot: Expr) -> Expr:
    return ls.replace(
      pendingBlocks=ls.pendingBlocks.replace(slot, List(Block))
    )

def ScheduleEventTimeout(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion3.ScheduleEventTimeout(param)

def Broadcast(_state: AgavePoolState, param: Expr) -> Expr:
    return _QuintUnion3.Broadcast(param)

def surpassesThreshold(_state: AgavePoolState, votes: Expr, threshold: Expr) -> Expr:
    return ((voting_power(_state, votes) * Val(100))
      >=
      (totalVotingPower(_state) * threshold))

def byzMessages(_state: AgavePoolState) -> Expr:

    def _quint_aux_1(quintTupledLambdaParam4122):
        hash = quintTupledLambdaParam4122[1]
        slot = quintTupledLambdaParam4122[0]
        return Set(
          _QuintUnion1.NotarVoteMsg(Record(slot=slot, hash=hash)),
          _QuintUnion1.NotarFallBackVoteMsg(Record(slot=slot, hash=hash)),
          _QuintUnion1.SkipVoteMsg(slot),
          _QuintUnion1.SkipFallbackVoteMsg(slot),
          _QuintUnion1.FinalVoteMsg(slot)
        )
    return AllTuples(_state.aliveSlots, _state.aliveHashes).map(_quint_aux_1).flattened

def byzNetworkMsgs(_state: AgavePoolState) -> Expr:

    def _quint_aux_2(quintTupledLambdaParam4145):
        msg = quintTupledLambdaParam4145[1]
        sender = quintTupledLambdaParam4145[0]
        return Record(sender=sender, msg=msg)
    return AllTuples(_state.byzantine, byzMessages(_state)).map(_quint_aux_2)

def isCertified(_state: AgavePoolState, cert: Expr, msgs: Expr) -> Expr:

    def _quint_aux_3(m):
        return UnionExpr(cert.node).match(
          FastFinalizationCertificate=(lambda b:
              ((m.msg == _QuintUnion1.NotarVoteMsg(b)))),
          NotarizationCertificate=(lambda b:
              ((m.msg == _QuintUnion1.NotarVoteMsg(b)))),
          NotarFallbackCertificate=(lambda b:
              (Or(
                  (m.msg == _QuintUnion1.NotarVoteMsg(b)),
                  (m.msg == _QuintUnion1.NotarFallBackVoteMsg(b))
                ))),
          SkipCertificate=(lambda s:
              (Or(
                  (m.msg == _QuintUnion1.SkipVoteMsg(s)),
                  (m.msg == _QuintUnion1.SkipFallbackVoteMsg(s))
                ))),
          FinalizationCertificate=(lambda s:
              ((m.msg == _QuintUnion1.FinalVoteMsg(s))))
        )
    aggregatedVotes = msgs.filter(_quint_aux_3)
    threshold = UnionExpr(cert.node).match(
      FastFinalizationCertificate=(lambda _: (Val(80))),
      NotarizationCertificate=(lambda _: (Val(60))),
      NotarFallbackCertificate=(lambda _: (Val(60))),
      SkipCertificate=(lambda _: (Val(60))),
      FinalizationCertificate=(lambda _: (Val(60)))
    )
    return surpassesThreshold(
      _state,
      aggregatedVotes.map((lambda x: (x.sender))),
      threshold
    )

def setTimeouts(_state: AgavePoolState, ls: Expr, slot: Expr) -> Expr:
    return Record(
      post=ls,
      output=windowSlots(_state, slot).map(
        (lambda i: (_QuintUnion3.ScheduleEventTimeout(i)))
      )
    )

def tryFinal(_state: AgavePoolState, ls: Expr, slot: Expr, hash: Expr) -> Expr:
    return Ite(
      And(
        And(
          And(
            ls.state[slot].contains(SlotObject.BlockNotarized(hash)),
            ls.state[slot].contains(SlotObject.VotedNotar(hash))
          ),
          ~ls.state[slot].contains(SlotObject.BadWindow())
        ),
        ~ls.state[slot].contains(SlotObject.ItsOver())
      ),
      Record(
        output=Set(_QuintUnion3.Broadcast(_QuintUnion1.FinalVoteMsg(slot))),
        post=addObjects(_state, ls, slot, Set(SlotObject.ItsOver()))
      ),
      Record(output=Set(ConsensusOutput), post=ls)
    )

def slotsSkipCertified(_state: AgavePoolState, msgs: Expr) -> Expr:

    def _quint_aux_4(s, x):
        return UnionExpr(x.msg.node).match(
          SkipVoteMsg=(lambda m: ((s | Set(m)))),
          SkipFallbackVoteMsg=(lambda m: ((s | Set(m)))),
          NotarVoteMsg=(lambda _: (s)),
          NotarFallBackVoteMsg=(lambda _: (s)),
          FinalVoteMsg=(lambda _: (s))
        )
    return msgs.reduce(_quint_aux_4, Set(int)).filter(
      (lambda s: (isCertified(_state, _QuintUnion2.SkipCertificate(s), msgs)))
    )

def finalized(_state: AgavePoolState, slot: Expr, msgs: Expr, blocks: Expr) -> Expr:
    return blocks.map((lambda b: (reference(_state, b)))).filter(
      (lambda b: ((b.slot == slot)))
    ).filter(
      (lambda b:
          (And(
              isCertified(
                _state,
                _QuintUnion2.FinalizationCertificate(b.slot),
                msgs
              ),
              isCertified(_state, _QuintUnion2.NotarizationCertificate(b), msgs)
            )))
    ).map((lambda b: (b.hash)))

def trySkipWindow(_state: AgavePoolState, ls: Expr, slot: Expr) -> Expr:

    def _quint_aux_5(s, k):
        return Ite(
          ~s.post.state[k].contains(SlotObject.Voted()),
          (lambda update:
              ((lambda s2: (Record(output=(s.output | update), post=s2)))(
                  clearPendingBlocks(
                    _state,
                    addObjects(
                      _state,
                      s.post,
                      k,
                      Set(SlotObject.Voted(), SlotObject.BadWindow())
                    ),
                    k
                  )
                )))(Set(_QuintUnion3.Broadcast(_QuintUnion1.SkipVoteMsg(k)))),
          s
        )
    return windowSlots(_state, slot).reduce(
      _quint_aux_5,
      Record(output=Set(_QuintUnion3), post=ls)
    )

def tryNotar(_state: AgavePoolState, ls: Expr, b: Expr) -> Expr:

    def _quint_aux_6(firstSlot):
        return Ite(
          Or(
            And(
              firstSlot,
              ls.state[b.slot].contains(SlotObject.ParentReady(b.parent))
            ),
            And(
              ~firstSlot,
              ls.state[(b.slot - Val(1))].contains(
                SlotObject.VotedNotar(b.parent)
              )
            )
          ),
          (lambda out:
              ((lambda s2:
                    ((lambda tf:
                          (Record(
                              result=Record(
                                post=tf.post,
                                output=(out | tf.output)
                              ),
                              success=Val(True)
                            )))(tryFinal(_state, s2, b.slot, b.hash))))(
                  clearPendingBlocks(
                    _state,
                    addObjects(
                      _state,
                      ls,
                      b.slot,
                      Set(SlotObject.Voted(), SlotObject.VotedNotar(b.hash))
                    ),
                    b.slot
                  )
                )))(
            Set(
              _QuintUnion3.Broadcast(
                _QuintUnion1.NotarVoteMsg(Record(slot=b.slot, hash=b.hash))
              )
            )
          ),
          Record(
            result=Record(post=ls, output=Set(ConsensusOutput)),
            success=Val(False)
          )
        )
    return Ite(
      ls.state[b.slot].contains(SlotObject.Voted()),
      Record(
        result=Record(post=ls, output=Set(ConsensusOutput)),
        success=Val(False)
      ),
      _quint_aux_6(firstSlotInLeaderWindow(_state, b.slot))
    )

def checkPendingBlocks(_state: AgavePoolState, ls: Expr) -> Expr:

    def _quint_aux_7(res, pendingBlocks):
        return pendingBlocks.reduce(
          (lambda res2, b:
              ((lambda tn:
                    (Record(
                        output=(res2.output | tn.result.output),
                        post=tn.result.post
                      )))(tryNotar(_state, res2.post, b)))),
          res
        )
    return ls.pendingBlocks.reduce(
      _quint_aux_7,
      Record(output=Set(_QuintUnion3), post=ls)
    )

def consensus(_state: AgavePoolState, ls: Expr, input: Expr) -> Expr:

    def _quint_aux_8(b):
        tn = tryNotar(_state, ls, b)
        return Ite(
          cast(BoolExpr, tn.success),
          (lambda cpb:
              (Record(post=cpb.post, output=(cpb.output | tn.result.output))))(
            checkPendingBlocks(_state, tn.result.post)
          ),
          Ite(
            ~tn.result.post.state[b.slot].contains(SlotObject.Voted()),
            Record(
              output=tn.result.output,
              post=appendPendingBlock(_state, tn.result.post, b.slot, b)
            ),
            Record(output=tn.result.output, post=tn.result.post)
          )
        )

    def _quint_aux_9(sh):
        s1 = addObjects(
          _state,
          ls,
          sh.slot,
          Set(SlotObject.BlockNotarized(sh.hash))
        )
        return tryFinal(_state, s1, sh.slot, sh.hash)

    def _quint_aux_10(sh):
        s1 = addObjects(
          _state,
          ls,
          sh.slot,
          Set(SlotObject.ParentReady(sh.hash))
        )
        s2 = checkPendingBlocks(_state, s1)
        s3 = setTimeouts(_state, s2.post, sh.slot)
        return Record(output=(s2.output | s3.output), post=s3.post)

    def _quint_aux_11(sh):
        s1 = trySkipWindow(_state, ls, sh.slot)
        return Ite(
          And(
            ~s1.post.state[sh.slot].contains(SlotObject.ItsOver()),
            ~s1.post.state[sh.slot].contains(
              SlotObject.VotedNotarFallback(sh.hash)
            )
          ),
          (lambda output:
              (Record(
                  output=output,
                  post=addObjects(
                    _state,
                    s1.post,
                    sh.slot,
                    Set(
                      SlotObject.BadWindow(),
                      SlotObject.VotedNotarFallback(sh.hash)
                    )
                  )
                )))(
            (s1.output
              |
              Set(_QuintUnion3.Broadcast(_QuintUnion1.NotarFallBackVoteMsg(sh))))
          ),
          s1
        )

    def _quint_aux_12(slot):
        s1 = trySkipWindow(_state, ls, slot)
        return Ite(
          And(
            ~s1.post.state[slot].contains(SlotObject.ItsOver()),
            ~s1.post.state[slot].contains(SlotObject.VotedSkipFallback())
          ),
          (lambda output:
              (Record(
                  output=output,
                  post=addObjects(
                    _state,
                    s1.post,
                    slot,
                    Set(SlotObject.BadWindow(), SlotObject.VotedSkipFallback())
                  )
                )))(
            (s1.output
              |
              Set(
                _QuintUnion3.Broadcast(_QuintUnion1.SkipFallbackVoteMsg(slot))
              ))
          ),
          s1
        )
    return UnionExpr(input.node).match(
      BlockInput=_quint_aux_8,
      TimeOutInput=(lambda slot:
          (Ite(
              ~ls.state[slot].contains(SlotObject.Voted()),
              trySkipWindow(_state, ls, slot),
              Record(output=Set(_QuintUnion3), post=ls)
            ))),
      BlockNotarizedInput=_quint_aux_9,
      ParentReadyInput=_quint_aux_10,
      SafeToNotarInput=_quint_aux_11,
      SafeToSkipInput=_quint_aux_12
    )

def votedToSkip(_state: AgavePoolState, s: Expr, msgs: Expr) -> Expr:
    return msgs.filter((lambda m: ((m.msg == _QuintUnion1.SkipVoteMsg(s))))).map(
      (lambda m: (m.sender))
    )

def votedToNotar(_state: AgavePoolState, b: Expr, msgs: Expr) -> Expr:
    return msgs.filter((lambda m: ((m.msg == _QuintUnion1.NotarVoteMsg(b))))).map(
      (lambda m: (m.sender))
    )

def parentNotarFallbackCertified(_state: AgavePoolState, b: Expr, msgs: Expr, blocks: Expr) -> Expr:
    return Or(
      (b.parent == -Val(1)),
      blocks.exists(
        (lambda p:
            (And(
                (p.hash == b.parent),
                isCertified(
                  _state,
                  _QuintUnion2.NotarFallbackCertificate(reference(_state, p)),
                  msgs
                )
              )))
      )
    )

def isGenHashOfOtherSlot(_state: AgavePoolState, slot: Expr, hash: Expr) -> Expr:
    return And((hash >= Val(100)), (((hash - Val(100)) / Val(10)) != slot))

def numSlots(_state: AgavePoolState) -> IntExpr:
    return (Val(4)
      *
      ((_state.aliveSlots.reduce((lambda m, x: (max(_state, m, x))), Val(0)) / Val(4))
        +
        Val(1)))

def genHash(_state: AgavePoolState, slot: Expr, lane: Expr) -> Expr:
    return ((Val(100) + (Val(10) * slot)) + lane)

def genesisBlock(_state: AgavePoolState) -> Expr:
    return Record(slot=-Val(1), hash=-Val(1), parent=-Val(1))

def frontierSlots(_state: AgavePoolState, ls: Expr) -> Expr:
    unvoted = _state.aliveSlots.filter(
      (lambda slot: (~ls.state[slot].contains(SlotObject.Voted())))
    )
    return Ite(
      (unvoted.size == Val(0)),
      _state.aliveSlots,
      Set(unvoted.reduce((lambda m, x: (min(_state, m, x))), numSlots(_state)))
    )

def byzSoup(_state: AgavePoolState) -> Expr:

    def _quint_aux_13(m):
        return UnionExpr(m.msg.node).match(
          NotarVoteMsg=(lambda r:
              (~isGenHashOfOtherSlot(_state, r.slot, r.hash))),
          NotarFallBackVoteMsg=(lambda r:
              (~isGenHashOfOtherSlot(_state, r.slot, r.hash))),
          SkipVoteMsg=(lambda _: (Val(True))),
          SkipFallbackVoteMsg=(lambda _: (Val(True))),
          FinalVoteMsg=(lambda _: (Val(True)))
        )
    return byzNetworkMsgs(_state).filter(_quint_aux_13)

def latestPrimaries(_state: AgavePoolState, kind: Expr, slot: Expr) -> Expr:
    return Range(Val(0), cast(IntExpr, slot)).reduce(
      (lambda acc, earlier:
          (Ite(
              And(kind.keys.contains(earlier), (kind[earlier] != Val(6))),
              Tuple(genHash(_state, earlier, Val(0)), acc[0]),
              acc
            ))),
      Tuple(-Val(1), -Val(1))
    )

def parentReadyBlocks(_state: AgavePoolState, slot: Expr, msgs: Expr, blocks: Expr) -> Expr:
    return Ite(
      ~firstSlotInLeaderWindow(_state, slot),
      Set(Block),
      (lambda certified:
          (Ite(
              And(
                (slot > Val(0)),
                (Set(Val(0), ..., (slot - Val(1)))
                  <=
                  slotsSkipCertified(_state, msgs))
              ),
              (certified | Set(genesisBlock(_state))),
              certified
            )))(
        blocks.filter(
          (lambda b:
              (And(
                  (b.slot < slot),
                  Or(
                    isCertified(
                      _state,
                      _QuintUnion2.NotarizationCertificate(reference(_state, b)),
                      msgs
                    ),
                    isCertified(
                      _state,
                      _QuintUnion2.NotarFallbackCertificate(
                        reference(_state, b)
                      ),
                      msgs
                    )
                  ),
                  (Set((b.slot + Val(1)), ..., (slot - Val(1)))
                    <=
                    slotsSkipCertified(_state, msgs))
                )))
        )
      )
    )

def applyEffect(_state: AgavePoolState, env: Expr, v: Expr, res: Expr) -> Expr:
    new = env.replace(system=env.system.replace(v, res.post))

    def _quint_aux_14(s, o):
        return UnionExpr(o.node).match(
          Broadcast=(lambda msg:
              ((lambda nm: (s.replace(msgBuffer=(s.msgBuffer | Set(nm)))))(
                  Record(sender=v, msg=msg)
                ))),
          ScheduleEventTimeout=(lambda slot:
              (s.replace(
                  activeTimeouts=s.activeTimeouts.replace(
                    v,
                    (s.activeTimeouts[v] | Set(slot))
                  )
                )))
        )
    return res.output.reduce(_quint_aux_14, new)

def safeToNotarHolds(_state: AgavePoolState, v: Expr, b: Expr, msgs: Expr, parentCertified: Expr) -> Expr:

    def _quint_aux_15(now):
        voted = msgs.filter(
          (lambda m: (And((m.sender == v), (now == slotOf(_state, m.msg)))))
        )
        notarVoted = voted.filter(
          (lambda m:
              (UnionExpr(m.msg.node).match(
                  NotarVoteMsg=(lambda r: ((r == reference(_state, b)))),
                  NotarFallBackVoteMsg=(lambda _: (Val(False))),
                  SkipVoteMsg=(lambda _: (Val(False))),
                  SkipFallbackVoteMsg=(lambda _: (Val(False))),
                  FinalVoteMsg=(lambda _: (Val(False)))
                )))
        )
        return And((voted.size > Val(0)), (notarVoted.size == Val(0)))

    def _quint_aux_16(notar):
        skip = votedToSkip(_state, b.slot, msgs)
        return Or(
          surpassesThreshold(_state, notar, Val(40)),
          And(
            surpassesThreshold(_state, (skip | notar), Val(60)),
            surpassesThreshold(_state, notar, Val(20))
          )
        )
    return And(
      _quint_aux_15(b.slot),
      _quint_aux_16(votedToNotar(_state, reference(_state, b), msgs)),
      Or(firstSlotInLeaderWindow(_state, b.slot), parentCertified)
    )

def safeToSkipHolds(_state: AgavePoolState, v: Expr, now: Expr, msgs: Expr) -> Expr:

    def _quint_aux_17(blocksWithNotarVotes):
        allNotarVotes = blocksWithNotarVotes.map(
          (lambda b: (votedToNotar(_state, b, msgs)))
        ).flattened
        blockWithMostVotes = blocksWithNotarVotes.reduce(
          (lambda res, x:
              (Ite(
                  (votedToNotar(_state, x, msgs).size
                    >
                    votedToNotar(_state, res, msgs).size),
                  x,
                  res
                ))),
          Record(slot=-Val(1), hash=-Val(1))
        )
        return surpassesThreshold(
          _state,
          ((votedToSkip(_state, now, msgs) | allNotarVotes)
            -
            votedToNotar(_state, blockWithMostVotes, msgs)),
          Val(40)
        )

    def _quint_aux_18(s, m):
        return UnionExpr(m.msg.node).match(
          NotarVoteMsg=(lambda n: (setAdd(_state, s, n))),
          NotarFallBackVoteMsg=(lambda _: (s)),
          SkipVoteMsg=(lambda _: (s)),
          SkipFallbackVoteMsg=(lambda _: (s)),
          FinalVoteMsg=(lambda _: (s))
        )
    return And(
      msgs.exists(
        (lambda m: (And((m.sender == v), (slotOf(_state, m.msg) == now))))
      ),
      ~msgs.exists(
        (lambda m:
            (And((m.sender == v), (m.msg == _QuintUnion1.SkipVoteMsg(now)))))
      ),
      _quint_aux_17(
        msgs.filter((lambda m: ((slotOf(_state, m.msg) == now)))).reduce(
          _quint_aux_18,
          Set(BlockReference)
        )
      )
    )

def allMessages(_state: AgavePoolState) -> Expr:
    return (_state.s.msgBuffer | byzSoup(_state))

def redundantDelivery(_state: AgavePoolState, ls: Expr, slot: Expr) -> Expr:

    def _quint_aux_19(b):
        return cast(BoolExpr, ls.pendingBlocks[slot].reduce(
          (lambda found, p: (Or(found, (p == b)))),
          Val(False)
        ))
    return Or(
      ls.state[slot].contains(SlotObject.Voted()),
      _state.s.blocks.filter((lambda b: ((b.slot == slot)))).forall(
        _quint_aux_19
      )
    )

def poolView(_state: AgavePoolState, msgs: Expr, blocks: Expr) -> Expr:

    def _quint_aux_20(slot):
        return msgs.filter((lambda m: ((slotOf(_state, m.msg) == slot))))
    msgsBySlot = (_state.aliveSlots | blocks.map((lambda b: (b.slot)))).map_to(
      _quint_aux_20
    )

    def _quint_aux_21(r):
        return Set(
          _QuintUnion2.FastFinalizationCertificate(r),
          _QuintUnion2.NotarizationCertificate(r),
          _QuintUnion2.NotarFallbackCertificate(r)
        ).filter((lambda c: (isCertified(_state, c, msgsBySlot[r.slot]))))
    blockCertificates = blocks.map((lambda b: (reference(_state, b)))).map(
      _quint_aux_21
    ).flattened
    parentCertified = blocks.map_to(
      (lambda b: (parentNotarFallbackCertified(_state, b, msgs, blocks)))
    )

    def _quint_aux_22(slot):
        return Set(
          _QuintUnion2.SkipCertificate(slot),
          _QuintUnion2.FinalizationCertificate(slot)
        ).filter((lambda c: (isCertified(_state, c, msgsBySlot[slot]))))
    slotCertificates = _state.aliveSlots.map(_quint_aux_22).flattened

    def _quint_aux_23(slot):
        return parentReadyBlocks(_state, slot, msgs, blocks).map(
          (lambda b: (Tuple(slot, b.hash)))
        )

    def _quint_aux_24(v):
        return blocks.filter(
          (lambda b:
              (safeToNotarHolds(
                  _state,
                  v,
                  b,
                  msgsBySlot[b.slot],
                  parentCertified[b]
                )))
        ).map((lambda b: (reference(_state, b))))

    def _quint_aux_25(v):
        return _state.aliveSlots.filter(
          (lambda slot: (safeToSkipHolds(_state, v, slot, msgsBySlot[slot])))
        )
    return Record(
      certificates=(blockCertificates | slotCertificates),
      parentReady=(Set(Tuple(Val(0), -Val(1)))
        |
        _state.aliveSlots.map(_quint_aux_23).flattened),
      safeToNotar=benevolent(_state).map_to(_quint_aux_24),
      safeToSkip=benevolent(_state).map_to(_quint_aux_25)
    )

def primaryParent(_state: AgavePoolState, kind: Expr, slot: Expr) -> Expr:
    k = kind[slot]
    latest = latestPrimaries(_state, kind, slot)
    return Ite(
      (k <= Val(5)),
      Ite(
        And((slot > Val(0)), firstSlotInLeaderWindow(_state, slot)),
        latestPrimaries(_state, kind, ((slot - Val(3)) + (k % Val(4))))[0],
        latest[0]
      ),
      Ite(
        (k == Val(7)),
        Ite((slot == Val(0)), -Val(1), genHash(_state, (slot - Val(1)), Val(9))),
        Ite((k == Val(8)), latest[1], -Val(1))
      )
    )

def safeToNotarCondition(_state: AgavePoolState, v: Expr, now: Expr, b: Expr) -> Expr:
    return And(
      (b.slot == now),
      safeToNotarHolds(
        _state,
        v,
        b,
        allMessages(_state),
        parentNotarFallbackCertified(
          _state,
          b,
          allMessages(_state),
          _state.s.blocks
        )
      )
    )

def finalizedBlocks(_state: AgavePoolState) -> Expr:
    return _state.aliveSlots.map_to(
      (lambda slot:
          (finalized(_state, slot, allMessages(_state), _state.s.blocks)))
    )

def safeToSkipCondition(_state: AgavePoolState, v: Expr, now: Expr) -> Expr:
    return safeToSkipHolds(_state, v, now, allMessages(_state))

def withPoolView(_state: AgavePoolState, env: Expr) -> Expr:
    return Ite(
      (_state.byzantine == Set(str)),
      env.replace(
        pool=poolView(_state, (env.msgBuffer | byzSoup(_state)), env.blocks)
      ),
      env
    )

def notarizedBlocksCondition(_state: AgavePoolState, v: Expr, b: Expr) -> Expr:
    return isCertified(
      _state,
      _QuintUnion2.NotarizationCertificate(b),
      allMessages(_state)
    )

def generateBlocks(_state: AgavePoolState, kind: Expr, equiv: Expr) -> Expr:

    def _quint_aux_26(slot):
        parent = primaryParent(_state, kind, slot)
        latest = latestPrimaries(_state, kind, slot)
        primary = Record(
          slot=slot,
          hash=genHash(_state, slot, Val(0)),
          parent=parent
        )
        second = Record(
          slot=slot,
          hash=genHash(_state, slot, Val(1)),
          parent=parent
        )
        e = equiv[slot]
        return Ite(
          (e == Val(0)),
          Set(primary, second),
          Ite(
            (e == Val(1)),
            Set(
              primary,
              second.replace(
                parent=Ite((parent == latest[0]), latest[1], latest[0])
              )
            ),
            Set(primary)
          )
        )
    return kind.keys.filter((lambda slot: ((kind[slot] != Val(6))))).map(
      _quint_aux_26
    ).flattened

@action(inline=False)
def initWith(c: Context[AgavePoolState], blocks: Expr):
    s = c.state

    def _quint_aux_27(_):
        return Record(
          state=listMap(
            s,
            Range(Val(0), numSlots(s)),
            (lambda i:
                (Ite(
                    (i == Val(0)),
                    Set(SlotObject.ParentReady(-Val(1))),
                    Set(SlotObject)
                  )))
          ),
          pendingBlocks=listMap(
            s,
            Range(Val(0), numSlots(s)),
            (lambda _: (List(Block)))
          )
        )
    s.s = withPoolView(
      s,
      Record(
        blocks=blocks,
        pool=Record(
          certificates=Set(_QuintUnion2),
          parentReady=Set(tuple[(int, int)]),
          safeToNotar=Map(str, set[BlockReference]),
          safeToSkip=Map(str, set[int])
        ),
        system=benevolent(s).map_to(_quint_aux_27),
        msgBuffer=Set(NetworkMsg),
        activeTimeouts=benevolent(s).map_to(
          (lambda v: (windowSlots(s, Val(0))))
        )
      )
    )
    s.ch = s.aliveSlots.map_to((lambda s: (Set(int))))
    s.counter = Val(0)

@action(inline=False)
def fireTimeoutEvent(c: Context[AgavePoolState], v: Expr, now: Expr):
    s = c.state
    c.assume(s.s.activeTimeouts[v].contains(now))
    s2 = s.s.replace(
      activeTimeouts=s.s.activeTimeouts.replace(
        v,
        (s.s.activeTimeouts[v] - Set(now))
      )
    )
    s.s = withPoolView(
      s,
      applyEffect(
        s,
        s2,
        v,
        consensus(s, s2.system[v], _QuintUnion4.TimeOutInput(now))
      )
    )
    s.ch = finalizedBlocks(s)
    s.counter = (s.counter + Val(1))

@action(inline=False)
def processInput(c: Context[AgavePoolState], id: Expr, input: Expr):
    s = c.state
    s.s = withPoolView(
      s,
      applyEffect(s, s.s, id, consensus(s, s.s.system[id], input))
    )
    s.ch = finalizedBlocks(s)
    s.counter = (s.counter + Val(1))

@action(inline=False)
def parentReadyAction(c: Context[AgavePoolState], v: Expr, now: Expr):
    s = c.state
    bs = parentReadyBlocks(s, now, allMessages(s), s.s.blocks)
    c.assume((bs.size > Val(0)))
    with c.one_of(bs, 'b') as b:
        processInput(
          c,
          v,
          _QuintUnion4.ParentReadyInput(Record(slot=now, hash=b.hash))
        )

@action(inline=False)
def safeToSkipAction(c: Context[AgavePoolState], v: Expr, now: Expr):
    s = c.state
    c.assume(safeToSkipCondition(s, v, now))
    processInput(c, v, _QuintUnion4.SafeToSkipInput(now))

@action(init=True)
def init(c: Context[AgavePoolState]):
    s = c.state
    with c.one_of(Set(allBlocks(s)), 'blocks') as blocks:
        initWith(c, blocks)

@action(inline=False)
def receiveSpecificBlock(c: Context[AgavePoolState], v: Expr, b: Expr):
    s = c.state
    c.assume(s.s.blocks.contains(b))
    processInput(c, v, _QuintUnion4.BlockInput(b))

@action(inline=False)
def safeToNotarAction(c: Context[AgavePoolState], v: Expr, now: Expr):
    s = c.state
    bs = s.s.blocks.filter((lambda b: (safeToNotarCondition(s, v, now, b))))
    c.assume((bs.size > Val(0)))
    with c.one_of(bs, 'b') as b:
        processInput(
          c,
          v,
          _QuintUnion4.SafeToNotarInput(Record(slot=b.slot, hash=b.hash))
        )

@action(inline=False)
def receiveBlock(c: Context[AgavePoolState], v: Expr, now: Expr):
    s = c.state
    with c.one_of(s.s.blocks.filter((lambda b: ((b.slot == now)))), 'block') as block:
        receiveSpecificBlock(c, v, block)

@action(inline=False)
def blockNotarizedAction(c: Context[AgavePoolState], v: Expr, now: Expr):
    s = c.state
    bs = s.s.blocks.filter(
      (lambda b:
          (notarizedBlocksCondition(s, v, Record(slot=now, hash=b.hash))))
    )
    c.assume((bs.size > Val(0)))
    with c.one_of(bs, 'b') as b:
        processInput(
          c,
          v,
          _QuintUnion4.BlockNotarizedInput(Record(slot=b.slot, hash=b.hash))
        )

@action(inline=False)
def messageResponse(c: Context[AgavePoolState], v: Expr, slot: Expr):
    s = c.state
    alts = iter(
      c.alternatives(
        'receiveBlock',
        'blockNotarizedAction',
        'parentReadyAction',
        'safeToNotarAction',
        'safeToSkipAction'
      )
    )
    with next(alts):
        receiveBlock(c, v, slot)
    with next(alts):
        blockNotarizedAction(c, v, slot)
    with next(alts):
        parentReadyAction(c, v, slot)
    with next(alts):
        safeToNotarAction(c, v, slot)
    with next(alts):
        safeToSkipAction(c, v, slot)

@action(inline=False)
def step(c: Context[AgavePoolState]):
    s = c.state
    with c.one_of(benevolent(s), 'v') as v:
        with c.one_of(Set(Val(0), ..., Val(1)), 'slotCoin') as slotCoin:
            with c.one_of(
                Ite(
                  (slotCoin == Val(0)),
                  s.aliveSlots,
                  frontierSlots(s, s.s.system[v])
                ),
                'slot'
              ) as slot:
                with c.one_of(Set(Val(0), ..., Val(3)), 'timeoutCoin') as timeoutCoin:
                    with c.one_of(Set(Val(0), ..., Val(3)), 'redeliveryCoin') as redeliveryCoin:
                        alts = iter(c.alternatives('actionAny', 'actionAll'))
                        with next(alts):
                            inner_alts = iter(
                              c.alternatives(
                                'actionAll',
                                'blockNotarizedAction',
                                'parentReadyAction',
                                'safeToNotarAction',
                                'safeToSkipAction'
                              )
                            )
                            with next(inner_alts):
                                c.assume(
                                  Or(
                                    (redeliveryCoin == Val(0)),
                                    ~redundantDelivery(s, s.s.system[v], slot)
                                  )
                                )
                                receiveBlock(c, v, slot)
                            with next(inner_alts):
                                blockNotarizedAction(c, v, slot)
                            with next(inner_alts):
                                parentReadyAction(c, v, slot)
                            with next(inner_alts):
                                safeToNotarAction(c, v, slot)
                            with next(inner_alts):
                                safeToSkipAction(c, v, slot)
                        with next(alts):
                            c.assume((timeoutCoin == Val(0)))
                            fireTimeoutEvent(c, v, slot)

@action(inline=False)
def initGenerated(c: Context[AgavePoolState]):
    s = c.state
    with c.one_of(AllMaps(s.aliveSlots, Set(Val(0), ..., Val(9))), 'kind') as kind:
        with c.one_of(AllMaps(s.aliveSlots, Set(Val(0), ..., Val(7))), 'equiv') as equiv:
            with c.one_of(Set(generateBlocks(s, kind, equiv)), 'blocks') as blocks:
                initWith(c, blocks)

@action(inline=False)
def noTimeout(c: Context[AgavePoolState]):
    s = c.state
    alts = iter(c.alternatives('alt_0'))
    with next(alts):
        with c.one_of(benevolent(s), 'v') as v:
            with c.one_of(s.aliveSlots, 'slot') as slot:
                messageResponse(c, v, slot)
