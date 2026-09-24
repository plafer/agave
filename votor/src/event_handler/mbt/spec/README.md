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

The vendored files are identical to upstream. Any change made to match
agave's intended behavior is listed here, together with the reason for it.

(none yet)
