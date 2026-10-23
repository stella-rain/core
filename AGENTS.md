# Stella Rain: core

Pure-Rust deterministic simulation for Stella Rain, an action shooter for phones and computers:
fixed-tick simulation, the stage, event and replay types, and the `stage-verify` CLI that
proves a stage can be cleared. Crate `stella-rain-core` (`stella_rain_core`), MIT or Apache-2.0.

## This repository is public

- Private planning (ads, moderation strategy, roadmap) goes in `stella-rain/app` issues, never here.
- Architecture decisions live in the private app repository; cite them by area ID (ADR-SIM-1).
  Rules about core's own behaviour (determinism, schema, input validation) may be summarised
  here; nothing else from the app repository is copied in.

## Hard rules

- No dependency on Godot or gdext; `core` builds and tests with plain `cargo` (ADR-BRDG-1).
- Deterministic: the same stage, seed and inputs give the same state hash every tick, on x86_64
  and arm64 (ADR-SIM-1, ADR-SIM-3). Details in `.claude/rules/determinism.md`, `simulation.md`.
- Stage files are untrusted: size limit before parsing, strict parsing, budgets, runtime
  clamps, fail closed (ADR-SVC-1).
- The Rust types here are the only schema; JSON Schema is generated from them (ADR-CONT-1).
  Any change to stage, event or replay types, or to a simulation outcome, follows the
  `schema-change` skill.
- Released content IDs (assets, palettes, parts, blocks, skills, attack patterns) are never renamed or removed, only deprecated (ADR-CONT-2).
- ADRs marked *Proposed* are the current plan: build on them, but raise a departure with Kade rather than depart silently.
- Design: standard practice first; give Kade options, trade-offs, a pick; better ideas as options.

## Layout

| Path | What |
|---|---|
| `src/` | The library: simulation, fixed-point math, RNG, collision, stage and replay types, validation, content, hashes |
| `src/rules_v0/` | The rules of `sim_version` 0, run by the engine |
| `src/bin/` | `stage-verify` (re-simulates a replay, checks the clear and the final hash) and `content-pack` |
| `tests/` | Integration tests in one binary (`integration.rs`); golden hashes, `insta` snapshots, `proptest` invariants |
| `tests/corpus/` | The replay corpus per `sim_version`; every tick's hash is checked (ADR-SIM-3) |
| `benches/` | `iai-callgrind` instruction counts of worst-case ticks (ADR-SIM-5) |
| `fuzz/` | `cargo-fuzz` targets for the parsers and loaders; its own workspace on nightly |
| `schema/` | JSON Schema generated from the types |
| `content/` | The bundled content, JSON read as data |
| `.github/` | Workflows and the scripts they run |
| `clippy.toml` | Bans `f32`, `f64`, `HashMap`, `HashSet` (ADR-SIM-3) |

What is inside these folders loads from `.claude/rules/` when those files are touched.
## Gates

| Part | Gate |
|---|---|
| Rust code | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` |
| Simulation | The replay corpus on x86_64 and arm64 (ADR-SIM-3, ADR-PLAT-1); an intended change: `UPDATE_CORPUS=1 cargo test --test integration corpus::` |
| iOS readiness | `cargo check --target aarch64-apple-ios` on every PR (ADR-PLAT-1) |
| Parser, validator, replay loader, share codes, run setups | In `fuzz/`: fmt, clippy, `cargo test`, `cargo +nightly fuzz build`; a short fuzz run per target in CI |
| Tick cost | `cargo bench --bench worst_tick --bench collide_tick --bench time_limit_tick`; CI fails on more than +2% instructions (ADR-SIM-5) |
| `.github/scripts/*.py` | `python3 -m unittest discover -s .github/scripts` (CI: `scripts.yml`) |
| `AGENTS.md`, `CLAUDE.md`, `.claude/` | `python3 ../.github/scripts/claude_md_check.py .` (CI runs it too); `AGENTS.md` at most 100 lines (`wc -l`) |
| Line endings | CI `eol-check` fails on any CRLF file (`git ls-files --eol`) |

The corpus run on an Android device is a release gate (ADR-REL-3), tried on the prerelease, and so are the
iOS Simulator and long fuzz runs, which Candidate runs; when a gate is needed and cannot run
here, add `cmd:verify-needs-android` (or `-windows`, `-macos`) to the issue.
In CI (`rust.yml`) a PR of only `*.md` files skips every Rust job; `rust checks` passes when each job ran or was
skipped by right. Fuzz, bench and workflow detail: `.claude/rules/benches-fuzz.md`, `ci.md`.

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
  `.github/workflows/`, `AGENTS.md`, `CLAUDE.md` or `.claude/rules/` are left to Kade: open such a
  PR as a draft, and he merges it; `branch-cleanup.yml` deletes their merged branches. Any other branch (Kade's own) is merged by hand. A local session opens
  the PR with Kade's `gh` login; without it, it prints the commands for Kade.
  `auto-merge.yml` is read from `main`, so a change to it acts only after its own merge.
- Releases follow the `release-flow` skill. The version is `version` in `Cargo.toml` (`fuzz/Cargo.lock`
  agrees); Candidate generates the release notes from the commits since the last release (`.github/scripts/release.py`).
- Releases are tags; `app` pins a tag, so a schema change needs a core tag before the app can use it (ADR-REL-2). Zip deliveries (Windows PC only) are laid out from the `stella-rain` root.

## Overrides of global rules

None.

## Never commit

- `target/`, fuzz `artifacts/` and generated fuzz corpora (the replay corpus in `tests/corpus/` is committed).
