---
paths:
  - "content/**"
  - "schema/**"
  - "src/stage.rs"
  - "src/behaviour.rs"
  - "src/attack.rs"
  - "src/weapon.rs"
  - "src/input.rs"
  - "src/event.rs"
  - "src/snapshot.rs"
  - "src/validate.rs"
  - "src/registry.rs"
  - "src/run_setup.rs"
  - "src/rules.rs"
  - "src/run_stats.rs"
  - "src/rewards.rs"
  - "src/match_ups.rs"
  - "src/rune.rs"
  - "src/runes.rs"
  - "src/run_content.rs"
  - "src/pack.rs"
  - "src/attack_presets.rs"
  - "src/bin/content_pack.rs"
  - "modes/**"
---

# Types, validator and content

Sources: ADR-CONT-11, ADR-CONT-12, ADR-CONT-13, ADR-CONT-15, ADR-CONT-16, ADR-SVC-7, ADR-REPL-12,
ADR-REPL-13, ADR-REPL-16.
A change to a type, an event or a content ID starts with the `schema-change` skill.

## What is where

- The types (at `SCHEMA_VERSION`; version 1 is the frozen first version, ADR-REPL-13) are
  `stage.rs`, `behaviour.rs`, `attack.rs`, `weapon.rs`, `input.rs`, `event.rs` and
  `snapshot.rs` (ADR-REPL-12, ADR-SIM-4, ADR-REPL-16). `serde`, `serde_json` and `schemars` parse and
  describe them, outside the simulation path.
- `validate.rs`: size limit, strict parse, registry, static budgets (ADR-SVC-7).
  `registry.rs`: the content-ID registry (ADR-CONT-12).
- `run_setup.rs`: the run setup a run starts from (ADR-REPL-16); each mode's content is bundled
  in `modes/`. `offer_mode` is `stop` (the default, skipped when written) or `timed`, which needs
  `sim_version` 2; it is the run's own, so a replay names it (ADR-SIM-7 point 4).
- `run_stats.rs` (the stats a point raises) and `rewards.rs` (what a falling star's offer
  holds): JSON read as untrusted bytes, an open list over effect kinds in code, defaulting to
  `content/run_stats.json` and `content/rewards.json` (ADR-CONT-11). `match_ups.rs` reads
  `content/match_ups.json`; `rune.rs` is the closed list of runes and `runes.rs` reads
  `content/runes.json`, the same way.
- `run_content.rs`: a run's content, read once (ADR-CONT-11): rules, run stats, rewards,
  expertise, match-ups, runes, stage skills, the attack presets and the `trial` and
  `time_limit` mode files, each with the SHA-256 of its bytes and the pack's SHA-256
  (ADR-REPL-16, ADR-CONT-16). The engine, the recorder and the verifier read the same value.
- `pack.rs`, `src/bin/content_pack.rs`: the content pack, the game's content files as one JSON
  file, hashed as its bytes and read as untrusted bytes (ADR-SVC-7, ADR-CONT-16).
  `content-pack verify FILE` checks one; `content-pack export [DIR]` writes the pack (no DIR
  writes `Pack::BUNDLED`; DIR builds from `DIR/*.json` and `DIR/../modes`).
- `content/`: `pack.json` is a copy byte for byte of a `stella-rain/content` release
  (ADR-CONT-15); `rules.json`, `run_stats.json`, `rewards.json`, `stage_skills.json`,
  `expertise.json`, `match_ups.json` and `runes.json` come out of `Pack::BUNDLED`
  (`ContentFiles::bundled_file`). `attack_presets.json` (the attack presets, ADR-CONT-13 point 6) is a
  required member of the pack, read by `attack_presets.rs`; a check that needs a preset's
  phases runs at pack load and in `World::new`.
- `schema/`: generated from the types (`pack.schema.json` is the content pack's);
  `cargo xtask bless` regenerates it (and the corpus, goldens and `content/pack.json`).

## Content or code (ADR-CONT-13)

- A number a designer tunes for feel or balance is content: a strict, bounded, schema-generated key
  of `rules.json`, read once into `Tuning`. A `tune/` PR changes it: a new pack hash, no new `sim_version`.
- Caps, the grid, the push-apart, the view and the units stay in Rust. The test: a number whose
  change needs code review stays in Rust. A key's bounds are code (`validate_rules` in `rules.rs`; the two ranged
  token pools sum to at most `MAX_RANGED_ATTACKERS`).
- `offer_timer_ticks` of `rules.json`: a `timed` offer's countdown, 1 to `MAX_OFFER_TIMER_TICKS`
  (3,600 = 60 s), checked in `validate_rules`, 300 when absent, never read by a `stop` run. A key
  added after bytes were published carries `#[serde(default)]`, so an older pack still reads; a
  setup key also `skip_serializing_if` its default, so an older setup writes the same bytes.
- `docs/tuning.md` (generated from `src/limits.rs`) lists caps and code numbers only; a key's
  bounds are the schema's.
