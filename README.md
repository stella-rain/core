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

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
