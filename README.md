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
