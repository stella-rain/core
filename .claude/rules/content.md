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
  - "src/run_stats.rs"
  - "src/rewards.rs"
  - "src/match_ups.rs"
  - "src/rune.rs"
  - "src/runes.rs"
  - "src/run_content.rs"
  - "src/pack.rs"
  - "src/bin/content_pack.rs"
  - "modes/**"
---

# Types, validator and content

Sources: ADR-CONT-1, ADR-CONT-2, ADR-CONT-3, ADR-SVC-1, ADR-REPL-1, ADR-REPL-3. A change to a type,
an event or a content ID starts with the `schema-change` skill.

## What is where

- The version 0 types are `stage.rs`, `behaviour.rs`, `attack.rs`, `weapon.rs`, `input.rs`, `event.rs` and
  `snapshot.rs` (ADR-REPL-1, ADR-SIM-4, ADR-REPL-3). `serde`, `serde_json` and `schemars` parse and
  describe them, outside the simulation path.
- `validate.rs`: size limit, strict parse, registry, static budgets (ADR-SVC-1).
  `registry.rs`: the content-ID registry (ADR-CONT-2).
- `run_setup.rs`: the run setup a run starts from (ADR-REPL-3); each mode's content is bundled
  in `modes/`.
- `run_stats.rs` (the stats a point raises) and `rewards.rs` (what a falling star's offer
  holds): JSON read as untrusted bytes, an open list over effect kinds in code, defaulting to
  `content/run_stats.json` and `content/rewards.json` (ADR-CONT-1). `match_ups.rs` reads
  `content/match_ups.json`; `rune.rs` is the closed list of runes and `runes.rs` reads
  `content/runes.json`, the same way.
- `run_content.rs`: a run's content, read once (ADR-CONT-1): run stats, rewards, expertise,
  match-ups, runes, stage skills and the `trial` and `time_limit` mode files, each with the
  SHA-256 of its bytes and the pack's SHA-256 (ADR-REPL-3, ADR-CONT-3). The engine, the recorder
  and the verifier read the same value.
- `pack.rs`, `src/bin/content_pack.rs`: the content pack, the game's content files as one JSON
  file, hashed as its bytes and read as untrusted bytes (ADR-SVC-1, ADR-CONT-3).
  `content-pack verify FILE` checks one; `content-pack export` writes the bundled one.
- `content/`: `pack.json` is a copy byte for byte of a `stella-rain/content` release
  (ADR-CONT-3); `run_stats.json`, `rewards.json`, `stage_skills.json`, `expertise.json`,
  `match_ups.json` and `runes.json` are each read by their `src/` module (`include_bytes!`).
- `schema/`: generated from the types (`pack.schema.json` is the content pack's);
  `UPDATE_SCHEMA=1 cargo test --test integration schema::` regenerates it.
