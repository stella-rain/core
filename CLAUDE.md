# Stella Rain: core

Pure-Rust deterministic simulation for Stella Rain, a stage-creator vertical shooter:
fixed-tick simulation, the stage, event and replay types, and the `stage-verify` CLI that
proves a stage can be cleared. Crate `stella-rain-core` (`stella_rain_core`), MIT or Apache-2.0.

## This repository is public

- Private planning (ads, moderation strategy, roadmap) goes in `stella-rain/app` issues, never here.
- Architecture decisions live in the private app repository; cite them by number (ADR-004).
  Rules about core's own behaviour (determinism, schema, input validation) may be summarised
  here; nothing else from the app repository is copied in.

## Hard rules

- No dependency on Godot or gdext; `core` builds and tests with plain `cargo` (ADR-003).
- Deterministic: the same stage, seed and inputs give the same state hash every tick, on x86_64
  and arm64 (ADR-004, ADR-019). Details in `.claude/rules/determinism.md`.
- Stage files are untrusted: size limit before parsing, strict parsing, budgets, runtime
  clamps, fail closed (ADR-020).
- The Rust types here are the only schema; JSON Schema is generated from them (ADR-018).
  Any change to stage, event or replay types, or to a simulation outcome, follows the
  `schema-change` skill.
- Released content IDs (assets, palettes, parts, blocks, skills, attack patterns) are never
  renamed or removed, only deprecated (ADR-026).
- ADRs marked *Proposed* are the current plan: build on them, but raise a departure with Kade
  rather than depart silently.

## Layout

So far: the ADR-014 spike, the version 0 types, validator, engine, attacks, bosses and replays; the rest as it arrives.

| Path | What |
|---|---|
| `src/` | Library: simulation, fixed-point math, RNG, collision, blocks, stage parsing, validation, replay |
| `src/hash.rs`, `src/spike.rs` | The FNV-1a 64 state hash; the ADR-014 spike simulation (not a frozen `rules_vN`) |
| `src/content_hash.rs` | The stage content hash, SHA-256 of the two file hashes (ADR-034); `sha2` lives outside the simulation path |
| `src/stage.rs`, `behaviour.rs`, `attack.rs`, `input.rs`, `event.rs`, `snapshot.rs` | The version 0 types (ADR-035, ADR-036); `serde`, `serde_json` and `schemars` parse and describe them, outside the simulation path |
| `src/replay.rs`, `share.rs`, `base64.rs`, `recording.rs` | The `.replay` file and its packed inputs, record and verify (ADR-009, ADR-016); share codes (`miniz_oxide`, outside the simulation path); `recording` is the `record` feature's JSON Lines |
| `src/engine.rs`, `src/rules_v0/`, `src/trig.rs`, `src/presets.rs` | The engine (`Engine`: step, events, snapshot, hash); the rules of `sim_version` 0 (agents, attacks, skills, the main shot, boss phases and parts), which say what they do not do yet; the interpolated sine table; attack presets in the stage's own JSON |
| `src/validate.rs`, `src/registry.rs` | The validator (size limit, strict parse, registry, static budgets; ADR-020) and the content-ID registry (ADR-026) |
| `schema/` | JSON Schema generated from the types; `UPDATE_SCHEMA=1 cargo test --test schema` regenerates it |
| `tests/` | Integration tests; `spike.rs` and `engine.rs` pin golden hashes every platform must match; `golden.rs` keeps the recordings as `insta` snapshots in `tests/golden/` (`cargo insta test --features record --review`) |
| `clippy.toml` | Bans `f32`, `f64`, `HashMap`, `HashSet` (ADR-019) |
| `src/bin/stage_verify.rs` | `stage-verify`: re-simulates a replay and checks the clear and the final hash (ADR-009, ADR-015) |
| `tests/corpus/` | Replay corpus per `sim_version`: `v0/<case>/` holds `stage.json`, `run.replay`, `hashes.txt`; `corpus.rs` checks it (ADR-019) |
| `fuzz/` | `cargo-fuzz` targets: parser, validator, replay loader (ADR-020) |
| `benches/` | `criterion` tick-time benchmarks with worst-case budgets (ADR-021) |
| `.github/workflows/` | Rust gates on x86_64 and arm64, mobile `cargo check`, Project sync, `CLAUDE.md` check, auto-merge |

## Gates

| Part | Gate |
|---|---|
| Rust code | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` |
| Simulation | Replay corpus: every tick's hash matches, on x86_64 and on an arm64 runner, every PR (ADR-019); an intended change: `UPDATE_CORPUS=1 cargo test --test corpus` |
| iOS readiness | `cargo check --target aarch64-apple-ios` (ADR-014) |
| Parser, validator, replay loader | Fuzz targets build; a short fuzz run when parsing changed |
| `CLAUDE.md`, `.claude/` | `python3 ../.github/scripts/claude_md_check.py .` (CI runs it too) |
| Line endings | CI `eol-check` fails on any CRLF file (`git ls-files --eol`) |

The corpus run on an Android device and the long fuzz run are release gates (ADR-022); when they
are needed and cannot run here, add `cmd:verify-needs-android` (or `-windows`, `-macos`) to the issue.

## State

- Issues in this repo, with command labels (`cmd:status-now`, `cmd:verify-not-verified`, ...),
  sync to the organization Project. No `MEMORY.md`, no `.memory/`, no handoff files.
- The Project snapshot is `STATUS.md` on the `status` branch of `app`:
  `git -C ../app fetch origin status && git -C ../app show origin/status:STATUS.md`.
- What a change did, what was verified and the wrong turns: the PR, or the commit body.

## Version control

- **Local sessions** (on Kade's PC): commit each finished task to `main` automatically,
  without being asked. Kade pushes.
- **Cloud sessions**: the PR says `Closes #N`. `auto-merge.yml` (not GitHub auto-merge) merges
  it once every check on its head is green and deletes the branch. `auto-merge.yml` is read
  from `main`, so a change to it acts only after its own merge.
- Releases are tags; `app` pins a tag, so a schema change needs a core tag before the app
  can use it (ADR-029).
- Zip deliveries are laid out from the `stella-rain` root.

## Never commit

- `target/`, fuzz `artifacts/` and generated fuzz corpora (the replay corpus in
  `tests/corpus/` is committed).
