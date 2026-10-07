# Stella Rain: core

Pure-Rust deterministic simulation for Stella Rain, a stage-creator vertical shooter:
fixed-tick simulation, the stage, event and replay types, and the `stage-verify` CLI that
proves a stage can be cleared. Crate `stella-rain-core` (`stella_rain_core`), MIT or Apache-2.0.
Follow the `kade-workflow` skill; where it and this file differ, this file wins.

## This repository is public

- Everything here is public: code, history, issues, Actions logs.
- Private planning (ads, moderation strategy, roadmap) goes in `stella-rain/app` issues, never here.
- Architecture decisions live in the private app repository; cite them by number (ADR-004).
  Rules about core's own behaviour (determinism, schema, input validation) may be summarised
  here; nothing else from the app repository is copied in.
- No secrets, tokens, internal URLs or personal email addresses in code, fixtures or commits.

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

The crate is not written yet; create these as the code arrives.

| Path | What |
|---|---|
| `src/` | Library: simulation, fixed-point math, RNG, collision, blocks, stage parsing, validation, replay |
| `stage-verify` binary | CLI that re-simulates a replay and checks the clear (ADR-009, ADR-015) |
| `tests/corpus/` | Replay corpus with per-tick hashes, per `sim_version` (ADR-019) |
| `fuzz/` | `cargo-fuzz` targets: parser, validator, replay loader (ADR-020) |
| `benches/` | `criterion` tick-time benchmarks with worst-case budgets (ADR-021) |
| `.github/workflows/` | Project sync, `CLAUDE.md` check |

## Gates

Run the gate for every part touched. A gate that could not run is named in the commit body or
PR as `NOT VERIFIED: <gate>: <reason>`.

| Part | Gate |
|---|---|
| Rust code | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` |
| Simulation | Replay corpus: every tick's hash matches (once `tests/corpus/` exists) |
| Parser, validator, replay loader | Fuzz targets build; a short fuzz run when parsing changed |
| `CLAUDE.md`, `.claude/` | `python3 ../.github/scripts/claude_md_check.py .` (CI runs it too) |

The arm64 hash check and release fuzzing are release gates (ADR-022); when they are needed and
cannot run here, add `cmd:verify-needs-kade` to the issue.

## State

- Issues in this repo, with command labels (`cmd:status-now`, `cmd:verify-not-verified`, ...),
  sync to the organization Project. No `MEMORY.md`, no `.memory/`, no handoff files.
- The Project snapshot is `STATUS.md` on the `status` branch of `app`:
  `git -C ../app fetch origin status && git -C ../app show origin/status:STATUS.md`.
- What a change did, what was verified and the wrong turns: the PR, or the commit body.

## Version control

- **Local sessions** (on Kade's PC): commit each finished task to `main` automatically,
  without being asked. Kade pushes.
- **Cloud sessions**: branch `claude/<task>`, push, open a PR that says `Closes #N`; Kade
  merges. Never commit to `main`. One task per PR; wait for the merge before the next.
- Author: `Kade <23338687+enjay27@users.noreply.github.com>`. No other email in commits or git config.
- Subject: the finding or the point of the change (`kade-workflow` section 5).
- Releases are tags; `app` pins a tag, so a schema change needs a core tag before the app
  can use it (ADR-029).
- The remote file tools cannot write `.github/` or `.claude/` on Kade's PC. Deliver those
  files as a zip laid out from the `stella-rain` root; Kade extracts it, then commit.

## Never commit

- `target/`, fuzz `artifacts/` and generated fuzz corpora (the replay corpus in
  `tests/corpus/` is committed).
- Secrets of any kind.
