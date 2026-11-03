# The corpus and the golden hash on an iPhone

`core` runs its replay corpus and the engine's golden hash on a real iPhone, built and started from a Mac
with a Cargo runner, `scripts/ios_device_runner.py`, that uses Xcode's own tools (`xcrun devicectl`,
`codesign`, `security`). The iPhone 13 is the reference device. This is the iOS half of the device gate
(ADR-REL-3): the same simulation on the phone's own arm64, iOS loader and libc gives the same state hash on
every checked tick (ADR-SIM-3, ADR-PLAT-1).

## What runs, and what does not

- Run: the 25 tests of `corpus::` (every tick of each stage case and every 60th tick of each run case, in the
  blessed `tests/corpus/current/` and the frozen `tests/corpus/v1/`) and
  `engine::the_golden_hash_is_the_same_on_every_platform`: 26 tests.
- Not run: the rest of the `integration` binary, the doctests, `golden::` (the `insta` snapshots bake the
  Mac path in), `corpus_pins::` and `corpus_contract::` (the filter `corpus::` does not match them).
- Not built for iOS at all: the seven modules gated `cfg(not(target_os = "ios"))` because they start a host
  process or read a host path (`content_pack`, `content_pack_compare`, `content_pack_unpack`, `one_source`,
  `stage_verify`, `tuning`, `cwd_free_paths`): 51 tests.

Tests find the repository through `data_root()` in `tests/data_root.rs`: the folder in
`STELLA_TEST_DATA_ROOT`, else the app bundle's folder, else `CARGO_MANIFEST_DIR`. The runner puts
`Cargo.toml`, `content/`, `schema/` and `tests/` (about 6 MB) into the app, so `tests/corpus/` is there on the
phone. Scratch files go to the phone's temp folder (`scratch_dir()`).

## Mac setup, once

1. Xcode, with `xcode-select -s /Applications/Xcode.app` and its license accepted. `python3` is the one from
   Xcode's command line tools (3.9 or later; the runner uses only the standard library).
2. `rustup target add aarch64-apple-ios` (the repository's `rust-toolchain.toml` lists it already).
3. Sign in to Xcode with an Apple ID. Create an empty iOS project named **Dinghy** with the bundle ID
   `com.stellarain.Dinghy`, run it once on the phone, and trust the developer certificate on the phone
   (Settings, General, VPN & Device Management). This creates the provisioning profile the runner reads. A
   free Apple ID's profile lasts 7 days: run the Dinghy project again from Xcode when it expires.
4. Developer Mode on the phone (Settings, Privacy & Security, Developer Mode; it restarts the phone).
5. Connect the phone by USB, unlock it, and accept "Trust This Computer". Keep it unlocked during a run.
   Connect one iOS device at a time (an Apple Watch or a Simulator is ignored).

Nothing else to install: no cargo-dinghy, no pymobiledevice3, no `sudo`, no tunnel.

## The command

From a checkout of the commit to test:

```bash
cargo test --locked --all-features --test integration --release --target aarch64-apple-ios -- corpus:: the_golden_hash_is_the_same_on_every_platform
```

Debug is the same command without `--release`. `.cargo/config.toml` makes `scripts/ios_device_runner.py` the
runner of this exact triple (the Simulator triple has its own runner in CI); the path resolves from any
folder of the repository.

## What the runner does

1. Lists the devices with `xcrun devicectl list devices` and takes the one physical iOS device whose tunnel
   is connected. None, or more than one, stops it.
2. Reads the provisioning profiles of `~/Library/MobileDevice/Provisioning Profiles` and
   `~/Library/Developer/Xcode/UserData/Provisioning Profiles` (`security cms -D`) and takes the one for
   `com.stellarain.Dinghy` with the latest expiry that is not expired, lists this phone and has a certificate
   in the keychain (`security find-identity`).
3. Builds `Dinghy.app` in a temporary folder: the test binary, an `Info.plist`, the profile as
   `embedded.mobileprovision`, and the data folders above.
4. Signs it with `codesign` (the identity and entitlements of the profile) and verifies the signature.
5. Installs it (`devicectl device install app`) and launches it with `devicectl device process launch
   --console`, passing the test arguments, and prints the app's console as it arrives.
6. Removes the temporary folder and exits nonzero unless `devicectl` exited 0, the log says
   `The app terminated with the exit code 0.` and holds a `test result:` line and no `test result: FAILED`.
   `devicectl` itself exits 0 when the app fails, so the log is what decides.

## Reading the result

- `ios-runner: device <name>, iOS <version>` names the phone it picked. Every runner message starts with
  `ios-runner:`.
- The last test line should be
  `test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; N filtered out`, followed by
  `The app terminated with the exit code 0.` and a zero exit status. `26 passed` moves only when the corpus
  grows. `N` is the iOS binary's total minus 26 and moves with every added test: it is smaller than on a Mac
  (1859 at one point, from a host count) because the seven modules above are not built for iOS. It was 1808
  on 2026-10-31 at `14ab0a2`. Do not compare it with a host run; read the passed count: `ok. 0 passed` is
  not a failure to Cargo or to the runner (a filter that matched nothing).
- A failing `corpus::the_corpus_matches` names the first diverging tick of the case and writes both snapshots
  to the phone's temp folder; report the message.

| Symptom (runner message) | Cause and fix |
|---|---|
| `no physical iPhone is connected (saw: ...)` | Not connected, not trusted or the tunnel is down: unlock, replug, accept the trust prompt. |
| `more than one physical iPhone is connected` | Disconnect all but one. |
| `no provisioning profile for com.stellarain.Dinghy` | Setup step 3 was not done: run the **Dinghy** project once from Xcode. |
| `the provisioning profile ... expired on <date>` | A free Apple ID's profile lasts 7 days: run the Dinghy project from Xcode again. |
| `the provisioning profile does not list this iPhone` | A new phone: run the Dinghy project on it. |
| `no signing identity in the profile is in your keychain` | Xcode, Settings, Accounts, Manage Certificates. |
| `install failed` with a locked-phone message | Message not seen. Unlock the phone and keep it unlocked. |
| `install failed` or `launch failed` about Developer Mode | Message not seen. Setup step 4. |
| The app does not launch and iOS says the developer is not trusted | Happens after a reinstall replaced the Xcode-installed app: Settings, General, VPN & Device Management, trust it again. |
| `the app exited with code N` | A test or the process failed: read the log above it. |
| `no test result line` | The test harness ended before a result (a bad argument such as `--test-threads=0`): read the log above it. |
| `no exit code` | The console ended without the app's exit: the phone was unplugged or locked mid-run, or `devicectl` timed out. Run again. |

## Host variables

- `STELLA_TEST_DATA_ROOT`: the root the tests read repository files from (a copy of the tree, for example).
- `STELLA_TEST_TMP_DIR`: the folder for scratch files.

`devicectl` passes the caller's variables that start with `DEVICECTL_CHILD_` to the app with the prefix
removed. On a device run set them as `DEVICECTL_CHILD_STELLA_TEST_DATA_ROOT=...`, and likewise
`DEVICECTL_CHILD_PROPTEST_CASES` and `DEVICECTL_CHILD_RUST_BACKTRACE`.

## What is verified

- On the iPhone 13 (iOS 27.2, Xcode 27.2) at the runner's commit, each from `cargo test`: the command above,
  release, 26 passed and exit 0; the same in debug; `DEVICECTL_CHILD_STELLA_TEST_DATA_ROOT=/nonexistent`
  gives `test result: FAILED`, although `devicectl` reports app exit code 0, and a nonzero exit; `--test-threads=0`
  gives no `test result:` line and a nonzero exit; the command run from `docs/`.
- On any host (CI `script unit tests`): the device and profile choice, the plists, the commands and the
  verdict, and whole runs against fake `xcrun`, `security` and `codesign`.
- On the host: the 26 tests pass from another working directory; with `STELLA_TEST_DATA_ROOT` at a copy whose
  `tests/corpus/current/runs/` is emptied, `corpus::the_corpus_matches` fails with `the corpus has no run case`.

Not verified: the real `devicectl` message for a locked phone and for Developer Mode off; a second iPhone;
a phone that is not on iOS 27.2; an Xcode other than 27.2 (the shape of `devicectl`'s JSON); the profile
after Xcode re-creates it; the 7-day expiry cycle itself. The device run cannot run in CI.
