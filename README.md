# Android CLI GUI

[![CI](https://github.com/RayhanHamada/android-cli-gui/actions/workflows/ci.yml/badge.svg)](https://github.com/RayhanHamada/android-cli-gui/actions/workflows/ci.yml)

Unofficial GUI for the [Android CLI](https://developer.android.com/tools/agents/android-cli).
Manage Android SDK platforms and build tools without installing Android Studio. Data
comes from the official Android CLI and Android SDK sources.

## Install

Homebrew (macOS 15+):

```sh
brew install RayhanHamada/tap/android-cli-gui
```

Prebuilt binaries for Linux, macOS (Apple Silicon and Intel), and Windows are on the
[releases page](https://github.com/RayhanHamada/android-cli-gui/releases).

The official `android` CLI must be installed and available on `PATH`.

## Build from source

Requires Rust stable and the [GPUI platform dependencies](https://gpui-kit.com/docs/installation/).

```sh
cargo run --release
```

## Releases

Releases are managed by [Knope Bot](https://knope.tech/tutorials/bot-workflow/).
Install the bot for this repository; it keeps a `knope/release` PR up to date.
Merging that PR creates a draft GitHub release, and the Release workflow builds
the binaries, uploads them, publishes the release, and updates the Homebrew tap.
