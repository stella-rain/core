---
paths:
  - "src/**"
  - "crates/**"
  - "tests/**"
  - "benches/**"
  - "fuzz/**"
  - "Cargo.toml"
  - "clippy.toml"
---

# Determinism rules

The same stage, seed and inputs must give the same state hash every tick, on x86_64 and arm64
(ADR-004, ADR-019). Replays are made on ARM phones and verified on x86 CI; a break here makes
published stages fail to verify.

## Simulation code and state

- Fixed timestep, 60 ticks per second. Nothing depends on frame time or the wall clock.
- Fixed-point integers for positions, velocities and angles (for example 1/256 pixel units).
  No `f32` or `f64` in simulation state; trigonometry through lookup tables.
- Explicit integer widths: no `usize` or `isize` in simulation state or in the hash.
- One PRNG per run, seeded from the stage: SplitMix64 written in `core` (ADR-033), never a
  crate. No other randomness: no thread RNG, time, pointer addresses or thread scheduling.
- Collision is our own code (circle vs point, circle vs circle); no engine physics.
- Entities live in `Vec`s with stable IDs. Never iterate a `HashMap` or `HashSet` in
  simulation code; use `Vec`, `BTreeMap` or sorted keys.
- `overflow-checks = true` in every Cargo profile, release included. Where overflow is possible,
  use explicit `saturating_*` or `wrapping_*` operations; any other overflow panics, the panic is
  caught at the gdext boundary and the run ends (fail closed).
- One input record per tick is the only external influence.
- A state hash per tick covers the full state. It is FNV-1a 64 over an explicit byte encoding
  that `core` writes itself; never `DefaultHasher` or a derived `Hash`.
- ADR-019 enforces the float, `usize`/`isize` and hash-iteration bans as lints (for example
  clippy `disallowed-types`); add them when the crate is created.

## `sim_version` (ADR-017)

- Version 0 is a development version (ADR-035): rules, numbers and the state's byte encoding
  change in place, without a `rules_vN` copy, until the freeze at P3. The x86_64 and arm64
  hash check is not relaxed: a PR that changes an outcome on purpose regenerates the version 0
  corpus and says `Corpus regenerated: <why>`; any other hash change is a bug. SplitMix64 and
  FNV-1a 64 stay fixed under version 0.
- From version 1 on, the rules below apply in full.
- Released `rules_vN` modules are frozen. A change that can alter an outcome goes into a new
  version through the `schema-change` skill.
- Shared helpers used by a frozen version keep their behaviour; copy before changing.
- Frozen rules include the state hash and its byte encoding (ADR-019), and the dynamic budget
  caps with their overflow rules (ADR-020): raising a cap is a new `sim_version`.

## Tests

- The corpus in `tests/corpus/` holds replays per `sim_version` with per-tick hashes. Every
  PR re-simulates it on x86_64 and on an arm64 runner; a mismatch reports the first diverging
  tick and dumps both snapshots. The Android device run is a release gate (ADR-022).
- `proptest` invariants: same input gives the same hash, HP never negative, buffs expire,
  positions stay in bounds.
- Budgets (ADR-020) are enforced twice, by the validator and by runtime clamps; test at the
  limit and one past it.
- A new dependency in the simulation path is checked for floats, hash ordering and
  platform-dependent behaviour first, and named in the commit body.
