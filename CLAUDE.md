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
- Design: standard practice first; give Kade options, trade-offs, a pick; better ideas as options.

## Layout

So far: the ADR-039 spike, the version 0 types, validator, engine, attacks, bosses and replays; the rest as it arrives.

| Path | What |
|---|---|
| `src/` | Library: simulation, fixed-point math, RNG, collision, blocks, stage parsing, validation, replay |
| `src/hash.rs`, `src/spike.rs` | The FNV-1a 64 state hash; the ADR-039 spike simulation (not a frozen `rules_vN`) |
| `src/content_hash.rs` | The stage content hash, SHA-256 of the two file hashes (ADR-034); `sha2` lives outside the simulation path |
| `src/parsed_stage_hash.rs` | The parsed-stage hash the blocklist uses: SHA-256 of a tag and the parsed stage's compact JSON without `id` (ADR-046); its bytes are pinned in `tests/parsed_stage_hash.rs` |
| `src/stage.rs`, `behaviour.rs`, `attack.rs`, `weapon.rs`, `input.rs`, `event.rs`, `snapshot.rs` | The version 0 types (ADR-035, ADR-036, ADR-050, ADR-055); `serde`, `serde_json` and `schemars` parse and describe them, outside the simulation path |
| `src/replay.rs`, `share.rs`, `base64.rs`, `recording.rs` | The `.replay` file and its packed inputs, record and verify (ADR-009, ADR-016); share codes (`miniz_oxide`, outside the simulation path); `recording` is the `record` feature's JSON Lines |
| `src/engine.rs`, `src/rules_v0/`, `src/trig.rs`, `src/presets.rs` | The engine (`Engine`: step, events, snapshot, hash); the rules of `sim_version` 0 (agents, attacks and their hit boxes, skills, the main character's movement, facing, evade, Main and Sub weapons and their skills, boss phases and parts), which say what they do not do yet; the interpolated sine table; attack and weapon presets in the stage's own JSON |
| `src/validate.rs`, `src/registry.rs` | The validator (size limit, strict parse, registry, static budgets; ADR-020) and the content-ID registry (ADR-026) |
| `schema/` | JSON Schema generated from the types; `UPDATE_SCHEMA=1 cargo test --test schema` regenerates it |
| `tests/` | Integration tests; `spike.rs` and `engine.rs` pin golden hashes every platform must match; `golden.rs` keeps the recordings as `insta` snapshots in `tests/golden/` (`cargo insta test --features record --review`); `properties.rs` checks `proptest` invariants on the corpus stages (`PROPTEST_CASES=2000` for a longer run) |
| `clippy.toml` | Bans `f32`, `f64`, `HashMap`, `HashSet` (ADR-019) |
| `src/bin/stage_verify.rs` | `stage-verify`: re-simulates a replay, checks the clear and the final hash, and prints both stage hashes (ADR-009, ADR-015, ADR-046) |
| `tests/corpus/` | Replay corpus per `sim_version`: `v0/<case>/` holds `stage.json`, `run.replay`, `hashes.txt`; `corpus.rs` checks it (ADR-019) |
| `benches/` | `worst_tick.rs`: `iai-callgrind` instruction count of a worst-case tick, every dynamic cap of ADR-020 reached (ADR-021); `worst_case/` builds the stage and `tests/worst_case.rs` checks it holds every cap (`Engine::load`); `collide_tick.rs` counts the worst friendly-bullet collision within the caps (`collide_case/`, `tests/collide_case.rs`) |
| `fuzz/` | `cargo-fuzz` targets for the stage parser, the run after validation, the replay loader and the share-code decoder (ADR-020); its own workspace on nightly; `tests/smoke.rs` runs them on stable, `regressions/<target>/` keeps the inputs of crashes that were fixed |
| `.github/workflows/` | Rust gates on x86_64 and arm64 (tests also on Windows and macOS), mobile `cargo check`, the fuzz jobs, the weekly long fuzz run (`fuzz-long.yml`) and the iOS Simulator tests (`ios-sim.yml`), both called by the release checks (`release-checks.yml`), the worst-tick benchmark, Project sync, `CLAUDE.md` check, auto-merge, branch cleanup, actionlint, the secret and key-file scan (`repo-check.yml`), the release (`release.yml`) and its script tests (`scripts.yml`) |
| `.github/scripts/` | `release_tag.py` (the release's version, required checks and notes) with tests; the iOS Simulator runner and the fuzz report |

## Gates

| Part | Gate |
|---|---|
| Rust code | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` |
| Simulation | Replay corpus: every tick's hash matches, on x86_64, on an arm64 runner, and (tests only) on Windows x86_64 and macOS arm64 runners, every PR and weekly on `main` (ADR-019, ADR-039); an intended change: `UPDATE_CORPUS=1 cargo test --test corpus` |
| iOS readiness | `cargo check --target aarch64-apple-ios` on every PR (ADR-039); the tests and the corpus run in the iOS Simulator by hand or from the release checks (`ios-sim.yml`, runner `.github/scripts/ios-sim-runner.sh`, ADR-054) |
| Parser, validator, replay loader, share codes | In `fuzz/`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, then `cargo +nightly fuzz build`; a PR that touches parsing also runs `cargo +nightly fuzz run <target> -- -max_total_time=60` for each of the four targets (CI: `rust.yml`); weekly, each target runs 7 minutes on a growing cached corpus, and a crash opens a hash-only issue (`fuzz-long.yml`, `.github/scripts/fuzz-report.sh`) |
| Tick cost | `cargo bench --bench worst_tick --bench collide_tick` (needs `valgrind` and `iai-callgrind-runner` 0.16.1); CI compares both with the base commit's on both architectures and fails on more than +2% instructions in either (`rust.yml`, ADR-058); `-- --save-baseline=base`, then `-- --baseline=base --callgrind-limits=ir=2%` runs the same comparison here |
| `.github/scripts/*.py` | `python3 -m unittest discover -s .github/scripts` (CI: `scripts.yml`) |
| `CLAUDE.md`, `.claude/` | `python3 ../.github/scripts/claude_md_check.py .` (CI runs it too) |
| Line endings | CI `eol-check` fails on any CRLF file (`git ls-files --eol`) |

The corpus run on an Android device is a release gate (ADR-054), and so are the iOS Simulator and long
fuzz runs (`release-checks.yml`, on a PR from `main` into `rc`); when a gate is needed and cannot run
here, add `cmd:verify-needs-android` (or `-windows`, `-macos`) to the issue.

## State

- Issues in this repo, with command labels (`cmd:status-now`, `cmd:verify-not-verified`, ...),
  sync to the organization Project. No `MEMORY.md`, no `.memory/`, no handoff files.
- The Project snapshot is `STATUS.md` on the `status` branch of `app`:
  `git -C ../app fetch origin status && git -C ../app show origin/status:STATUS.md`.
- What a change did, what was verified and the wrong turns: the PR, or the commit body.

## Version control

- **Local and cloud sessions** work on `claude/<task>`; a task may hold several commits and
  gets one PR that says `Closes #N`. `auto-merge.yml` (not GitHub auto-merge) merges it once
  every check on its head is green and deletes the branch. Drafts and PRs that touch
  `.github/workflows/` are left to Kade (ADR-052); `branch-cleanup.yml` deletes their merged
  branches (ADR-053). Any other branch (Kade's own) is merged by hand. A local session opens
  the PR with Kade's `gh` login; without it, it prints the commands for Kade.
  `auto-merge.yml` is read from `main`, so a change to it acts only after its own merge.
- Release candidates are pull requests from `main` into `rc` (ADR-054): never commit to `rc`, and
  no other head. Kade merges the PR by hand, then starts `release.yml` on `rc`: it checks the
  release checks and tags `v<Cargo.toml version>` with a GitHub Release (ADR-054). `auto-merge.yml`
  never touches it (it acts on `claude/*` heads into `main` only). A fix found on a candidate
  goes to `main` first.
- Releases are tags; `app` pins a tag, so a schema change needs a core tag before the app
  can use it (ADR-029).
- Zip deliveries (needed on the Windows PC only) are laid out from the `stella-rain` root.

## Never commit

- `target/`, fuzz `artifacts/` and generated fuzz corpora (the replay corpus in
  `tests/corpus/` is committed).
