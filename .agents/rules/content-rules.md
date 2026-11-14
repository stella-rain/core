---
globs:
  - "content/rules.json"
  - "schema/rules.schema.json"
  - "schema/threat_colours.schema.json"
  - "src/rules.rs"
  - "src/behaviour.rs"
  - "src/attack*.rs"
  - "src/threat_colours.rs"
  - "src/run_setup.rs"
  - "src/stage.rs"
  - "src/limits.rs"
  - "tests/rules.rs"
  - "tests/threat_colours.rs"
  - "tests/try_styles.rs"
  - "tests/unlocked_styles.rs"
  - "tests/common/mod.rs"
---

# Content keys: rules, enemy fields, threat colours

Sources: ADR-CONT-1, ADR-CONT-3, ADR-BRDG-4, ADR-SIM-9. Where each content file is read: `content.md`.

## Content or code (ADR-CONT-3)

- A number a designer tunes for feel or balance is content: a strict, bounded, schema-generated key
  of `rules.json`, read once into `Tuning`. A `tune/` PR changes it: a new pack hash, no new `sim_version`.
- Caps, the grid, the push-apart, the view and the units stay in Rust. The test: a number whose
  change needs code review stays in Rust. A key's bounds are code (`validate_rules` in `rules.rs`; the two ranged
  token pools sum to at most `MAX_RANGED_ATTACKERS`).
- `offer_timer_ticks` of `rules.json`: read by no run, kept for older packs: 1 to `MAX_OFFER_TIMER_TICKS` (3,600), checked in
  `validate_rules`, 300 when absent. A key added after bytes were published carries `#[serde(default)]`, so an older pack still
  reads; a setup key also `skip_serializing_if` its default, so an older setup writes the same bytes.
- `docs/tuning.md` (generated from `src/limits.rs`) lists caps and code numbers only; a key's
  bounds are the schema's.

## Combat keys of `rules.json`

- The 25 of `COMBAT_RULE_KEYS` (`tests/common/mod.rs`); what each does: `combat.md`. Each is optional, bounded in
  `validate_rules` (`rules.rs`) and off when absent; `sim_version` 2 only.
- Two groups are all or none: `perfect_evade_*` (five keys, among the 25) and `rally_*` (three, not in `COMBAT_RULE_KEYS`). The
  bundled `rules.json` ships the first and has no `rally_*` key.

## Enemy, boss and phase fields

All `#[serde(default, skip_serializing_if ..)]`; an absent field is off.
- Enemy and elite (`behaviour.rs`): `poise` 0 to `MAX_POISE` (1,000); `stagger_max` 1 to `MAX_STAGGER` (10,000, `limits.rs`);
  `staggered_ticks` 1 to `MAX_STAGGERED_TICKS` (600), only with `stagger_max`. The boss has the same two (`stage.rs`).
- Attack phase (`attack.rs`): `poise_damage` 0 to `MAX_POISE_DAMAGE` (1,000); `counterable`; `advance`, a combo step's, 1 px to
  `MAX_ADVANCE_DISTANCE` (`limits.rs`). `MAX_POISE` and `MAX_POISE_DAMAGE` are in `validate.rs`.

## Threat colours

- `threat_colours.rs`: an optional pack member `threat_colours` (`pack.rs`), at most `MAX_THREAT_COLOURS` (16) entries and
  `MAX_THREAT_COLOURS_BYTES` (4 KiB). The bundled pack has none.
- `Agent.threat_colour` and the boss's name an entry (`behaviour.rs`, `sim_version` 2); a companion has none. It is for the
  view: the simulation decides nothing from it.

## Setup and stage

- `RunSetup.unlocked_styles` (`run_setup.rs`) names the styles a run may pick (`sim_version` 2); `Stage.try_styles` (`stage.rs`)
  lets only the tutorial mode try styles, and the run builder drops it from every other mode. What they do: `weapon-tree.md`.
