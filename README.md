# Stella Rain Core

Deterministic simulation core for **Stella Rain**, an action shooter for phones and computers.
Pure Rust with no Godot dependency: fixed-tick simulation, stage and replay schemas,
and the `stage-verify` CLI that proves a stage can be cleared.

Architecture decisions are recorded in the app repository (ADR-003, ADR-004, ADR-009, ADR-029).

## How a release reaches the content repository

A published core release (and a manual run of the `Content publish` workflow with a ref) runs
`content-pack export` at that ref and opens the pull request `bot/publish-<ref>`, "Publish
core <version>", in the content repository. It sets `pack.json` and `core-version` there.
The flow is one way: content never writes back, and its release follows the merge of that
pull request.

## How a release is made

Candidate is run on `main` (Actions, Candidate, Run workflow) and makes the prerelease
`vX.Y.Z-rc.N`; Promote, run on the branch `rc/vX.Y.Z-rc.N`, tags the same commit `vX.Y.Z`.
The `version` in `Cargo.toml` is a floor, not the release: above the latest release it is the
release, at or below it the release is the next patch (`0.1.0` with `v0.1.0` out gives
`v0.1.1`), and Candidate stamps that version into a release commit on top of the `main` commit.
`main` itself is never written to, so its `Cargo.toml` can lag the latest release; the tag is
the version. A minor or major release needs a version PR.
After a final release exists, Promote sends a `core-release` `repository_dispatch` carrying the tag
(`client_payload.tag`) to the app repository, whose `core-bump` bot opens the bump PR; a prerelease
is never sent. A failed dispatch leaves the release and fails the run, and re-running the failed
job `notify` resends it.

## Balancing

Tuning numbers live in core under `content/` and `modes/`. Preset arguments have a minimum and a maximum in the registry and the schema (`minimum`/`maximum` under `AttackRef`); a pack or stage outside them is refused at load (for today's presets, a wind-up of 0 to 600 ticks). Balancing workflow:

1. **Compare numbers:** `content-pack compare --numbers-only BASE_DIR HEAD_DIR` verifies that two
   content checkouts differ only in numeric leaf values. Any change to keys, IDs, strings,
   booleans, or array lengths fails closed with exit status 1. In `tune/` branches, this check
   enables fast CI verification and auto-merge.
2. **Playtest with the app:** `python3 scripts/playtest.py [--app ../app] [--profile test-env] [--build]`
   runs `content-pack verify` on the pack, checks that the app's `godot-ext` core pin matches
   the core version, optionally compiles the debug build (`--features record`), and launches Godot
   with `--content-path` pointing to this checkout's pack and an isolated test profile.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.

## How a core merge reaches the app repository

Every push to `main` runs the `Bump app` workflow, which sends a `repository_dispatch` to the
app repository (`event_type: core-main`, `client_payload: {"sha": "<full commit SHA>"}`).
App's bot answers with a pull request that moves its core pin to that commit.
