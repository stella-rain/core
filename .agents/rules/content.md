---
globs:
  - "content/**"
  - "schema/**"
  - "src/stage.rs"
  - "src/behaviour.rs"
  - "src/attack*.rs"
  - "src/weapon.rs"
  - "src/input.rs"
  - "src/text.rs"
  - "src/event.rs"
  - "src/snapshot.rs"
  - "src/validate.rs"
  - "src/validate/**"
  - "src/registry.rs"
  - "src/run_setup.rs"
  - "src/rules.rs"
  - "src/run_stats.rs"
  - "src/rewards.rs"
  - "src/match_ups.rs"
  - "src/rune*.rs"
  - "src/run_content.rs"
  - "src/pack.rs"
  - "src/pack/**"
  - "src/bin/content_pack.rs"
  - "modes/**"
---

# Types, validator and content

Sources: ADR-CONT-1, ADR-CONT-2, ADR-CONT-3, ADR-CONT-5, ADR-CONT-6, ADR-SVC-1, ADR-REPL-1,
ADR-REPL-2, ADR-REPL-5.
A change to a type, an event or a content ID starts with the `schema-change` skill.

## What is where

- The types (at `SCHEMA_VERSION`; version 1 is the frozen first version, ADR-REPL-2) are
  `stage.rs`, `behaviour.rs`, `attack.rs`, `weapon.rs`, `input.rs`, `event.rs` and
  `snapshot.rs` (ADR-REPL-1, ADR-SIM-4, ADR-REPL-5). `serde`, `serde_json` and `schemars` parse and
  describe them, outside the simulation path. A `Boss` may opt in to `chase {speed, stop_distance}` and
  `arrives_at_tick` (`sim_version` 2 only: `validate.rs`, `sim/boss.rs`).
- `validate.rs`: size limit, strict parse, registry, static budgets (ADR-SVC-1).
  `registry.rs`: the content-ID registry (ADR-CONT-2).
- `run_setup.rs`: the run setup a run starts from (ADR-REPL-5); each mode's content is bundled
  in `modes/`. `RunSetup` has no `offer_mode` and no `build_mode`: a setup naming either is refused as an unknown field.
- `run_stats.rs` (the stats a point raises) and `rewards.rs` (what a falling star's offer
  holds): JSON read as untrusted bytes, an open list over effect kinds in code, defaulting to
  `content/run_stats.json` and `content/rewards.json` (ADR-CONT-1). `match_ups.rs` reads
  `content/match_ups.json`; `rune.rs` is the closed list of runes and `runes.rs` reads
  `content/runes.json`, the same way.
- `run_content.rs`: a run's content, read once (ADR-CONT-1): rules, run stats, rewards,
  expertise, match-ups, runes, stage skills, the attack presets and the `trial` and
  `time_limit` mode files, each with the SHA-256 of its bytes and the pack's SHA-256
  (ADR-REPL-5, ADR-CONT-6). The engine, the recorder and the verifier read the same value.
- `pack.rs`, `src/bin/content_pack.rs`: the content pack, the game's content files as one JSON
  file, hashed as its bytes and read as untrusted bytes (ADR-SVC-1, ADR-CONT-6).
  `content-pack verify FILE` checks one; `content-pack export [DIR]` writes the pack (no DIR
  writes `Pack::BUNDLED`; DIR builds from `DIR/*.json` and `DIR/../modes`).
- `content/`: `pack.json` is a copy byte for byte of a `stella-rain/content` release
  (ADR-CONT-5); `rules.json`, `run_stats.json`, `rewards.json`, `stage_skills.json`,
  `expertise.json`, `match_ups.json` and `runes.json` come out of `Pack::BUNDLED`
  (`ContentFiles::bundled_file`). `attack_presets.json` (the attack presets, ADR-CONT-3 point 6) is a
  required member of the pack, read by `attack_presets.rs`; a check that needs a preset's
  phases runs at pack load and in `World::new`.
  `threat_colours` is an optional member (`threat_colours.rs`; the bundled pack has none): `content-rules.md`.
- `text.rs`: UI text outside the hashed pack, `content/text.<locale>.json` per `SHIPPED_LOCALES` code (`ko`, `en`).
- `schema/`: generated from the types (`pack.schema.json` is the content pack's);
  `cargo xtask bless` regenerates it (and the corpus, goldens and `content/pack.json`).

What a content key may hold, and what stays in Rust (ADR-CONT-3): `content-rules.md`.
