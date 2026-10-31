# The corpus and the golden hash on an iPhone

`core` runs its replay corpus and the engine's golden hash on a real iPhone, built and started from a Mac
with [cargo-dinghy](https://github.com/sonos/dinghy). The iPhone 13 is the reference device. This is the
iOS half of the device gate (ADR-REL-3): the same simulation on the phone's own arm64, iOS loader and libc
gives the same state hash on every checked tick (ADR-SIM-3, ADR-PLAT-1).

## What runs, and what does not

- Run: the 25 tests of `corpus::` (every tick of each stage case and every 60th tick of each run case, in the
  blessed `tests/corpus/current/` and the frozen `tests/corpus/v1/`) and
  `engine::the_golden_hash_is_the_same_on_every_platform`: 26 tests.
- Not run: the rest of the `integration` binary, the doctests, `golden::` (the `insta` snapshots bake the
  Mac path in), `corpus_pins::` and `corpus_contract::` (the filter `corpus::` does not match them).

Tests find the repository through `data_root()` in `tests/data_root.rs`: the folder in
`STELLA_TEST_DATA_ROOT`, else the app bundle's folder (a copy of the crate directory), else
`CARGO_MANIFEST_DIR`. cargo-dinghy copies the crate directory (without `target/`) next to the test binary, so
`tests/corpus/` is there on the phone. Scratch files go to the phone's temp folder
(`scratch_dir()`).

## Mac setup, once

1. Xcode, with `xcode-select -s /Applications/Xcode.app` and its license accepted.
2. `rustup target add aarch64-apple-ios` (the repository's `rust-toolchain.toml` lists it already).
3. `python3 -m pip install -U pymobiledevice3` (dinghy uses it for iOS 17 and later).
4. `cargo install cargo-dinghy`.
5. Sign in to Xcode with an Apple ID. Create an empty iOS project named **Dinghy**, run it once on the
   phone, and trust the developer certificate on the phone (Settings, General, VPN & Device Management). A free
   Apple ID's certificate expires after about a week: run the Dinghy project again from Xcode.
6. Developer Mode on the phone (Settings, Privacy & Security, Developer Mode; it restarts the phone).
7. Connect the phone by USB, unlock it, and accept "Trust This Computer". Connect only one iPhone: the runner
   takes the first device it finds.

## The command

From a checkout of the commit to test:

```bash
cargo test --locked --all-features --test integration --release --target aarch64-apple-ios -- corpus:: the_golden_hash_is_the_same_on_every_platform
```

Debug is the same command without `--release`. `.cargo/config.toml` makes `cargo dinghy runner --` the runner of
this target. For iOS 17 and later dinghy asks for the Mac's `sudo` password (it starts a tunnel to the phone).

## Reading the result

- A line like `Targeting platform ... and device ...` names the phone it picked.
- The last test line should be
  `test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 1859 filtered out`
  (the filtered count is the binary's total minus 26 and moves when tests are added), and dinghy ends with a
  zero exit status (`returns: 0`; `Failed` otherwise).
- A failing `corpus::the_corpus_matches` names the first diverging tick of the case and writes both snapshots
  to the phone's temp folder; report the message.

| Symptom | Cause and fix |
|---|---|
| No device found | Phone not connected, locked or not trusted: unlock it, replug, accept the trust prompt. |
| No signing identity or provisioning profile | Step 5 was not done: run the **Dinghy** project once from Xcode. |
| The app is rejected or does not start after a week | The free certificate expired: run the Dinghy project again, trust it. |
| The tunnel asks for a password | Type the Mac's `sudo` password. |
| Developer Mode error | Step 6. |
| A scratch test fails on `create_dir_all` | The temp folder is not writable in the app. Do not work around it: report the output. |

## Host variables

- `STELLA_TEST_DATA_ROOT`: the root the tests read repository files from (a copy of the tree, for example).
- `STELLA_TEST_TMP_DIR`: the folder for scratch files.

dinghy for iOS 17 and later does not pass environment variables to the app, so these are for host runs and
runners that do pass them; the phone uses the bundle.

## What is verified

- Verified on the host: the 26 tests pass from another working directory; with `STELLA_TEST_DATA_ROOT` at a
  copy whose `tests/corpus/current/runs/` is emptied, `corpus::the_corpus_matches` fails with
  `the corpus has no run case`, and with a full copy it passes; the test code compiles for
  `aarch64-apple-ios` and `aarch64-apple-ios-sim`.
- From dinghy's documentation and source: runner mode, the bundle layout (the crate directory beside the test
  binary), the setup steps above.
- Not verified until the first run on the phone: that the `cargo install` release is as new as dinghy's
  `main`; that the install accepts thousands of loose files (about 10 MB); that the temp folder is writable
  in the app; that libtest's output comes back through the debugger.
