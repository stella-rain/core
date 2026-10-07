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
- One PRNG per run, seeded from the stage. No other randomness: no thread RNG, time, pointer
  addresses or thread scheduling.
- Collision is our own code (circle vs point, circle vs circle); no engine physics.
- Entities live in `Vec`s with stable IDs. Never iterate a `HashMap` or `HashSet` in
  simulation code; use `Vec`, `BTreeMap` or sorted keys.
- Integer overflow is explicit (`wrapping_*`, `checked_*`, `saturating_*` where intended), and
  `overflow-checks` is set identically in every Cargo profile.
- One input record per tick is the only external influence.
- A state hash per tick covers the full state.
- ADR-019 enforces the float and hash-iteration bans as lints (for example clippy
  `disallowed-types`); add them when the crate is created.

## `sim_version` (ADR-017)

- Released `rules_vN` modules are frozen. A change that can alter an outcome goes into a new
  version through the `schema-change` skill.
- Shared helpers used by a frozen version keep their behaviour; copy before changing.

## Tests

- The corpus in `tests/corpus/` holds replays per `sim_version` with per-tick hashes. Every
  commit re-simulates it; a mismatch reports the first diverging tick and dumps both snapshots.
- `proptest` invariants: same input gives the same hash, HP never negative, buffs expire,
  positions stay in bounds.
- Budgets (ADR-020) are enforced twice, by the validator and by runtime clamps; test at the
  limit and one past it.
- A new dependency in the simulation path is checked for floats, hash ordering and
  platform-dependent behaviour first, and named in the commit body.
