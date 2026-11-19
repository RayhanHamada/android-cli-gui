# Android CLI GUI

[![CI](https://github.com/RayhanHamada/android-cli-gui/actions/workflows/ci.yml/badge.svg)](https://github.com/RayhanHamada/android-cli-gui/actions/workflows/ci.yml)

Unofficial GUI for the [Android CLI](https://developer.android.com/tools/agents/android-cli).
Manage Android SDK platforms and build tools, inspect connected devices and
emulators, without installing Android Studio. Data comes from the official
Android CLI and Android SDK sources.

## Install

Homebrew cask (macOS 15+):

```sh
brew tap RayhanHamada/tap
brew trust --cask RayhanHamada/tap/android-cli-gui
brew install --cask RayhanHamada/tap/android-cli-gui
```

This installs `Android CLI GUI.app` in `/Applications` and the official
[`android-cli`](https://developer.android.com/tools/agents/android-cli) cask as
a dependency. Launch it from Applications, Launchpad, or Spotlight like any
other Mac app.

Homebrew 7 requires trusting third-party casks before installation. On older
Homebrew versions the `brew trust` step can be skipped.

### Gatekeeper

Release builds are ad-hoc signed, not notarized. Homebrew 7 quarantines
downloads, so macOS blocks the first launch. Approve the app once in
**System Settings → Privacy & Security → Open Anyway**.

Removing that prompt requires Apple Developer ID signing and notarization in
the release workflow.

If you installed the old formula, remove it before switching:

```sh
brew uninstall android-cli-gui
```

Prebuilt macOS disk images (Apple Silicon and Intel), Windows and Linux
archives, and checksums are on the
[releases page](https://github.com/RayhanHamada/android-cli-gui/releases).

## Roadmap

No dates; the order reflects priority. A feature appears only when the official
Android CLI exposes the data or action it needs.

| Status | Area | Scope |
| --- | --- | --- |
| Shipped | SDK versions | List installed and available platforms; install, update, and remove them through `android sdk`. |
| Shipped | Build tools | The same list and lifecycle operations for `build-tools/*` packages. |
| Shipped | Devices | List every AVD from `android emulator list --long`, merge attached devices from `adb devices -l`, and start or stop AVDs. |
| Shipped | macOS distribution | Launchable app bundle, Homebrew cask, DMG and zip release assets. |
| Next | Emulators | AVD details, create/remove, cold boot, and headless start options. |
| Next | Settings | Resolved SDK root, CLI and Java versions, and environment information; read-only first. |
| Next | Device workflows | `android device hub` mirroring and `android device remote` reservations. |
| Next | Distribution trust | Developer ID signing and Apple notarization so the cask opens without a Gatekeeper prompt. |
| Later | Project workflows | Create, build, run, and install projects; `describe` and APK analysis. |
| Later | Diagnostics | Screen capture, input, Compose previews, profiler, signals, and Studio bridge. |
| Later | Other platforms | First-class Linux and Windows installers and update flow. |

### Principles

- The official Android CLI remains the source of truth; the app does not edit
  SDK files or bundle the SDK.
- Prefer actions the CLI supports over reimplementing `adb`, the emulator, or
  Android build tooling.
- Keep the UI usable when the CLI or a platform tool is missing, and report the
  exact command needed to fix it.

## Build from source

Requires Rust stable and the [GPUI platform dependencies](https://gpui-kit.com/docs/installation/).

```sh
cargo run --release
```

To build a macOS app bundle:

```sh
cargo build --release
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)"
packaging/macos/make-app.sh target/release/android-cli-gui "$version" \
  "target/Android CLI GUI.app"
packaging/macos/make-dmg.sh "target/Android CLI GUI.app" "$version" \
  "target/android-cli-gui-v${version}-macos-arm64.dmg"
open "target/Android CLI GUI.app"
```

## Releases

Releases are managed by [Knope Bot](https://knope.tech/tutorials/bot-workflow/).
Install the bot for this repository; it keeps a `knope/release` PR up to date.
Merging that PR creates a draft GitHub release, and the Release workflow builds
the packages, uploads them, publishes the release, and updates the Homebrew
cask in the tap.
