---
name: schema-change
description: Change a Stella Rain stage, event or replay type, or anything that can alter a simulation outcome (rules, numbers, RNG use, math, what a content ID does), or add a content ID. Use before editing those types in stella-rain/core, or when the app needs a new field, event or content ID from core.
---

# Schema change

The Rust types in `stella-rain/core` are the only schema (ADR-018). Stages and replays already
published in creators' repositories must keep loading and verifying forever (ADR-017, ADR-026).
This file is identical in `app` and `core`; change both copies in the same task.

## 1. Classify the change

| Change | Bump | What else |
|---|---|---|
| Any of the changes below while the version is still 0 (ADR-035) | none | Change in place: no migration, no `rules_vN` copy, no deprecation. If outcomes change, regenerate the version 0 corpus and write `Corpus regenerated: <why>` in the PR |
| File structure of stages, events or recordings: a field added, removed, renamed or retyped | `schema_version` | Forward migration on load |
| Anything that can alter an outcome: rules, numbers, RNG use, math, what a content ID does | `sim_version` | A new frozen `rules_vN`; older versions untouched |
| New content ID (asset, palette, part, block, targeting, skill, attack pattern) | none | Registry entry; permanent once released |
| Rename or remove a released content ID | not allowed | Deprecate it instead |
| Refactor, docs, comments | none | The replay corpus proves the hashes are unchanged |

Not sure whether outcomes change? Run the replay corpus: any hash difference means `sim_version`
(under version 0: a corpus regeneration, named in the PR). Version 0 never relaxes determinism:
the corpus must still match on x86_64 and arm64, and SplitMix64, FNV-1a 64 and the content hash
stay fixed (ADR-019, ADR-033, ADR-034). Version 0 ends with the freeze at P3.

## 2. Test first

- `schema_version`: a test that loads a file of the previous version and asserts the migrated result.
- `sim_version`: a corpus replay recorded under the new version, and the existing corpus still
  passing under the old versions without changes.
- Version 0: tests for the new behaviour; the regenerated corpus passing on both architectures.
- If the change adds size or count (more entities, a longer list), adjust the budget in both the
  validator and the runtime clamp (ADR-020), with a test at the limit and one past it.

## 3. Change `core`

1. Edit the types. Unknown fields stay rejected.
2. `schema_version`: bump the constant and add a migration from the previous version. Files on
   disk are never rewritten in place.
3. `sim_version`: copy the current rules into a new `rules_vN`, change only the new copy, and
   add its arm to the dispatch. New stages always get the latest version.
4. Content ID: add it to the registry with the version that introduces it, its status
   (active or deprecated) and its asset source (ADR-027). Deprecated IDs stay loadable.
5. Regenerate the JSON Schema and commit its diff with the change.
6. If parsing changed, make sure the fuzz targets build and run them briefly.

## 4. Gates

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, the replay
corpus. Name any that could not run: `NOT VERIFIED: <gate>: <reason>`.

## 5. Reach the app

- Tag a `core` release; `app` pins the new tag in its `Cargo.toml` (ADR-029), in its own commit
  or PR that refers to the `core` change.
- Editor fields come from the generated JSON Schema; never hand-edit a copy. Validation is
  core's alone: GDScript calls it through gdext and never validates against the schema (ADR-018).
- Work spanning both repositories is an epic issue in `app` with a sub-issue in `core`.

## 6. Commit

The subject states the effect, for example
`Waves gain an optional delay_ticks; schema_version 2 migrates version 1 files with a delay of 0`.
The body names the version bumped and why, the migration, and the corpus result; under
version 0, whether the corpus was regenerated and why.
