---
paths:
  - "src/savepoint/**"
  - "tests/savepoint.rs"
  - "tests/savepoint_proof.rs"
  - "tests/fixtures/savepoint/**"
---

# The savepoint file and its pins

Sources: ADR-SAVE-4, ADR-REPL-5. Also `replay.md`, `state-hash.md`.

## Pins

A pinned format or hash is never re-pinned: a change is a new version.

- Savepoint: `tests/fixtures/savepoint/v<N>.sha256` per `SAVEPOINT_FORMAT` (now 3,
  `savepoint/file.rs`; `v1`, `v2` and `v3` are pinned and `tests/savepoint.rs` wants one per
  older format); `UPDATE_SAVEPOINT_PIN=1` refuses to overwrite an existing file. A file of format
  1 or 2 is refused as `Format`. The payload starts with a fixed 60-byte `PlayerStats` record
  (per slot `damage_dealt` u64, `kills` u32, `support` u64; `STATS_RECORD_LEN`, `ledger.rs`),
  unhashed and trusted like the replay prefix, and a slot outside the party may hold a leaver's
  totals (`decode_for` ignores the mask). Then the state bytes (`state-hash.md`: the presence
  byte, the generations, a block per present player, a `timed` run's deadlines). `restore` builds
  its fresh engine as the party the prefix log ends on, after its last join or leave
  (`presence_at_end`); a size mismatch is `Damaged` or `PrefixMismatch`.
