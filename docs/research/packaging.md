# Packaging and distribution facts

Ticket: `.scratch/diff-viewer-mvp/issues/13-packaging-research.md`. Researched 2026-10-01.
Terms (Comparison, Side, Change Set) are defined in `CONTEXT.md`. `gdv` is the placeholder binary name from the CLI ticket.

**Labels.** "Inferred" means worked out from code or from several sources, not stated by the owner of the fact. "UNVERIFIED" means I could not confirm it. Homebrew `main` links are as of commit `d6ca355` (2026-10-01), and line numbers on `main` may move.

## Recommendation

### Channels for the MVP

| Channel | Ship in MVP? | Why |
|---|---|---|
| **GitHub Releases**: per-arch tarballs for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, built by GitHub Actions | **Yes, the base everything else repackages** | All runners are free for public repos ([arm64 Linux GA](https://github.blog/changelog/2025-08-07-arm64-hosted-runners-for-public-repositories-are-now-generally-available/), [macOS labels](https://github.com/actions/runner-images/blob/57b93f2cf7eda14a6a71f3175d15218626cdbd4c/README.md#L24-L36)). Zed, GitComet and Zedis ship one artifact per arch instead of a universal binary (see [CI](#5-github-releases-and-ci)). |
| **AUR `gdv-bin`**, repackaging the Linux tarball | **Yes, Arch first** | The `-bin` suffix is the rule for prebuilt packages ([AUR rules](https://wiki.archlinux.org/title/AUR_submission_guidelines#Rules_of_submission)). The `gitcomet-bin` gpui git client is precedent ([AUR](https://aur.archlinux.org/packages/gitcomet-bin)). It avoids cloning the whole Zed repo on every user's machine. |
| **AUR `gdv`**, source PKGBUILD | Optional, cheap to add | `cargo fetch --locked` in `prepare()` also fetches git deps, so `build()` can run offline ([Arch Rust guidelines](https://manual.archlinux.page/package-guidelines/rust/), [cargo-fetch](https://doc.rust-lang.org/cargo/commands/cargo-fetch.html)). It needs `!lto` or `-ffat-lto-objects` (see [AUR](#2-aur)). |
| **Homebrew personal tap** (`<user>/homebrew-tap`) with a **formula** that installs the prebuilt binary, macOS and Linux | **Yes** | Formulae are not quarantined; casks are (inferred, see [Homebrew](#1-homebrew)). Casks that fail Gatekeeper were disabled from 2026-09-01 ([Homebrew 5.0.0](https://brew.sh/2025/11/12/homebrew-5.0.0/)). cargo-dist already generates exactly this kind of per-arch formula ([template](https://github.com/axodotdev/cargo-dist/blob/v0.33.0/cargo-dist/templates/installer/homebrew.rb.j2#L36-L47)). |
| `cargo install --locked --git <repo> --tag <v>` | Document it as the developer route; not a primary channel | It works with git deps ([cargo-install](https://doc.rust-lang.org/cargo/commands/cargo-install.html)). It clones the whole Zed repo, because shallow git deps are still unstable ([`-Zgit=shallow-deps`](https://doc.rust-lang.org/cargo/reference/unstable.html#git)). On macOS it needs full Xcode for Metal (see [CI](#5-github-releases-and-ci)). |
| crates.io | **No** | crates.io rejects packages with git-only deps ([specifying dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#multiple-locations)). `gpui` on crates.io is stuck at 0.2.2 (2025-10-22) and `gpui_platform` is not published there ([crates.io API](https://crates.io/api/v1/crates/gpui)). |
| homebrew/core | **No (not yet)** | A self-submission needs 225 stars, 90 forks or 90 watchers, and a repo at least 30 days old ([notability](https://docs.brew.sh/Package-Acceptance-Policy#notability)). Whether reviewers accept a git-pinned Zed dependency is UNVERIFIED. |
| Homebrew cask | **No** | Casks must pass Gatekeeper, which needs Developer ID signing plus notarization ([Acceptable Casks](https://docs.brew.sh/Acceptable-Casks#platform-compatibility-and-macos-security-protections)). |
| Flatpak, Snap | **No** | Both sandboxes break the core contract: run the user's `git`, read their dotfiles, and see `git difftool`'s `/tmp` files (see [sandboxes](#4-flatpak-appimage-snap)). |
| AppImage | Later, if non-Arch users ask | It works (no sandbox) but adds nothing over the tarball for a terminal-launched tool. It needs FUSE, or `--appimage-extract-and-run`. |

### Signing stance

**Ship unsigned for the MVP.** Do not pay the USD 99/year ([Apple](https://developer.apple.com/programs/enroll/)) until there is a reason to: a cask, a `.app`, or users who download from a browser.

- On Apple Silicon the linker ad-hoc signs every binary automatically. That is enough for the binary to run ([Big Sur notes](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes)), but ad-hoc signatures "cannot pass through Gatekeeper".
- Gatekeeper only checks quarantined files ([DTS](https://developer.apple.com/forums/thread/666452)).
  - `curl` does not quarantine ([DTS](https://developer.apple.com/forums/thread/706442)).
  - `tar` does not propagate quarantine (same thread; a forum statement, so verify it).
  - Homebrew quarantines only cask downloads (inferred from code, see below).
  - So the tap formula and `curl | tar` installs never meet Gatekeeper. A browser download extracted with Archive Utility does.
- The release notes must say: download with `curl`, or run `xattr -d com.apple.quarantine gdv`, or approve it once in System Settings → Privacy & Security. Since Sequoia the Control-click override is gone ([Apple](https://developer.apple.com/news/?id=saqachfa), [support](https://support.apple.com/en-us/102445)).
- If you sign later:
  - Use a Developer ID Application certificate, hardened runtime, a secure timestamp and `notarytool` ([Apple](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)).
  - A bare Mach-O can be notarized but **not stapled** ([Apple](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow#Staple-the-ticket-to-your-distribution)). Offline first launch then needs a DMG or `.pkg` wrapper.

### Minimum git version

| Floor | What it buys |
|---|---|
| **2.11.0** | Hard minimum to run at all. It adds `status --porcelain=v2` and `cat-file --batch` with `--textconv`/`--filters`. |
| **2.23.0** | Feature-complete: `blame.ignoreRevsFile` is honoured. |
| **2.30.0** (recommended supported floor) | `--ignore-revs-file` tolerates stale object names instead of failing the whole blame. |

- Every current platform is above 2.30: Debian 11 2.30.2, Ubuntu 22.04 2.34.1, Apple Git-154 2.39.5 (see [git versions](#7-minimum-git-version)).
- Do **not** use `cat-file --batch-command` (2.36) or `blame --contents <file> <rev>` (2.41). The design doesn't need either.

### macOS build and `.app`

- **Build:** keep gpui's default build-time Metal shaders. In CI, run on `macos-15` (default Xcode 16.4) and add an explicit `xcodebuild -downloadComponent MetalToolchain` step. Xcode 26 runner images intermittently lack the Metal toolchain ([runner-images#13014](https://github.com/actions/runner-images/issues/13014), [#14013](https://github.com/actions/runner-images/issues/14013)).
- **Fallback:** the `gpui_platform/runtime_shaders` feature, which needs no Xcode but compiles shaders at every app start; its startup cost is unmeasured.
- **No `.app` for the MVP.** gpui sets the activation policy and the menu bar in code ([platform.rs L1320, L1084](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_macos/src/platform.rs#L1320)). Verify on a Mac (ticket 09) before relying on it.

## 1. Homebrew

**Formula vs cask**
- "Use a formula for open source command-line software and libraries that Homebrew can build from source. Use a cask for native macOS applications and for proprietary or supported binary-only software." ([docs](https://docs.brew.sh/Adding-Software-to-Homebrew#choose-a-package-type))
- In homebrew/core, a formula "whose primary output is a native macOS `.app` bundle is not eligible" ([docs](https://docs.brew.sh/Acceptable-Formulae#native-macos-applications)). A GUI is allowed only as an optional part of a CLI formula ([docs](https://docs.brew.sh/Acceptable-Formulae#optional-graphical-interfaces)).
- gdv is a CLI that opens a window and builds no `.app`, so it is formula-shaped.
- Core formulae must build from source; "platform-specific binary-only software belongs in a cask" ([docs](https://docs.brew.sh/Acceptable-Formulae#source-availability-and-licences)). That rule binds official taps only.
- Third-party taps: "Software that does not meet the official criteria can generally be maintained in a third-party tap" ([docs](https://docs.brew.sh/Package-Acceptance-Policy#scope-and-third-party-taps)).
- No doc forbids or endorses a tap formula whose `url` is a release tarball. cargo-dist generates exactly that: per-OS and per-arch `url`/`sha256` chosen with `OS.mac?`/`Hardware::CPU.arm?` ([template L36-47, L92-95](https://github.com/axodotdev/cargo-dist/blob/v0.33.0/cargo-dist/templates/installer/homebrew.rb.j2#L36-L47)). Formulae also support `on_macos`/`on_linux`/`on_arm`/`on_intel` blocks ([Formula Cookbook](https://docs.brew.sh/Formula-Cookbook), "Handling different system configurations").

**Taps and trust**
- Create the tap with `brew tap-new user/homebrew-tap`. The repo needs the `homebrew-` prefix ([docs](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap#creating-a-tap)).
- Since Homebrew 6.0.0 (2026-06-11), third-party taps must be trusted. `brew install user/tap/gdv` trusts that one formula ([Tap Trust](https://docs.brew.sh/Tap-Trust), [6.0.0](https://brew.sh/2026/06/11/homebrew-6.0.0/)).

**Building from source with a git-pinned dependency**
- A formula may define `fetch` (network allowed), which runs `cargo fetch *std_cargo_fetch_args`. `install` then runs offline ([Formula Cookbook](https://docs.brew.sh/Formula-Cookbook#fetching-dependencies-before-building)).
- `std_cargo_fetch_args` is `--locked --target host-tuple`. `std_cargo_args` adds `--offline` when `fetch` is defined ([formula.rb L2205-2224](https://github.com/Homebrew/brew/blob/d6ca35509427ed7b4e37445054fa343ab90755ec/Library/Homebrew/formula.rb#L2205-L2224)).
- Sources must be immutable, and an install must not "resolve a moving or otherwise unreproducible dependency set" ([docs](https://docs.brew.sh/Acceptable-Formulae#versioned-and-verifiable-sources)). A tag-pinned git dependency locked in `Cargo.lock` appears to satisfy this (inferred; no rule mentions Cargo git deps).
- macOS source builds also need full Xcode for Metal (`depends_on xcode: :build`) unless `runtime_shaders` is used.

**Bottles**
- "Homebrew only builds and supports bottles for homebrew/core" ([docs](https://docs.brew.sh/Homebrew-Security-and-Supply-Chain#bottles-built-by-homebrew)).
- A `brew tap-new` tap gets GitHub Actions workflows that build bottles, published with `brew pr-pull` ([docs](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap#creating-a-tap)). `root_url` lets you host them yourself ([docs](https://docs.brew.sh/Bottles#root-url-root_url)).
- A prebuilt-tarball formula makes bottles unnecessary.
- Homebrew 7.0.0 (2026-09-13) moved Intel Macs to Tier 3: no routine Intel bottles ([7.0.0](https://brew.sh/2026/09/13/homebrew-7.0.0/)). One more reason to ship our own x86_64 tarball.

**Unsigned casks and quarantine**
- Homebrew 5.0.0 (2025-11-12): "Casks without codesigning are deprecated. We will disable all Homebrew/homebrew-cask casks that fail Gatekeeper checks in September 2026." `--no-quarantine` was deprecated at the same time ([5.0.0](https://brew.sh/2025/11/12/homebrew-5.0.0/), [brew#20755](https://github.com/Homebrew/brew/issues/20755)) and its leftovers removed on 2026-07-30 ([brew#23363](https://github.com/Homebrew/brew/pull/23363)).
- This is enforced: dmenu-mac was disabled on 2026-09-01 with reason `fails_gatekeeper_check` because it was ad-hoc signed ([homebrew-cask#290450](https://github.com/Homebrew/homebrew-cask/pull/290450)).
- Casks are quarantined: "Homebrew applies macOS quarantine attributes to Cask downloads" ([docs](https://docs.brew.sh/Homebrew-Security-and-Supply-Chain#casks-have-a-different-trust-model)). The quarantine call is in `cask/download.rb` ([L78](https://github.com/Homebrew/brew/blob/d6ca35509427ed7b4e37445054fa343ab90755ec/Library/Homebrew/cask/download.rb#L78)).
- **Formulae are not quarantined**: inferred from code. Quarantine logic lives only under `cask/`, and no doc says it outright.
- On Apple Silicon, `brew` ad-hoc re-signs Mach-O files it relocates ([keg.rb L108](https://github.com/Homebrew/brew/blob/d6ca35509427ed7b4e37445054fa343ab90755ec/Library/Homebrew/extend/os/mac/keg.rb#L108)).

**How gpui apps ship on Homebrew**
- Zed: official cask with per-arch DMGs; `binary "#{appdir}/Zed.app/Contents/MacOS/cli", target: "zed"` ([zed.rb](https://github.com/Homebrew/homebrew-cask/blob/HEAD/Casks/z/zed.rb)).
- GitComet: cask in its own tap. Zedis: official cask (both in [CI](#5-github-releases-and-ci)).
- All of these are signed and notarized `.app`s. None ships a bare CLI formula, so gdv's formula route has no direct gpui precedent.

## 2. AUR

**Package names** ([AUR submission guidelines](https://wiki.archlinux.org/title/AUR_submission_guidelines#Rules_of_submission))

| Suffix | Use |
|---|---|
| none | built from source at a specific version |
| `-bin` | "prebuilt deliverables, when the sources are available" |
| `-git` | tracks a branch, not a release |

- `x86_64` support is required.
- Use SPDX licence identifiers and install the licence file ([PKGBUILD#license](https://wiki.archlinux.org/title/PKGBUILD#license)).

**Rust guidelines** (now in the Arch Developer Manual: [manual.archlinux.page/package-guidelines/rust](https://manual.archlinux.page/package-guidelines/rust/))
- `prepare()`: `export RUSTUP_TOOLCHAIN=stable; cargo fetch --locked --target host-tuple`. This "enables the later build() and other stages to be run entirely offline".
- `build()`: `cargo build --frozen --release --all-features`.
- `cargo fetch` downloads "all of the git dependencies and/or registry dependencies" ([cargo-fetch](https://doc.rust-lang.org/cargo/commands/cargo-fetch.html)). Git deps are therefore fine for a source PKGBUILD, given a committed `Cargo.lock`.
- The Zed git dependency means a full Zed clone per build: shallow deps are unstable ([cargo unstable `git`](https://doc.rust-lang.org/cargo/reference/unstable.html#git)).

**LTO problem**
- makepkg's default `-flto=auto` breaks gpui's C shims with rust-lld. The `disktree` PKGBUILD uses `options=('!lto')` and explains why ([PKGBUILD](https://raw.githubusercontent.com/archlinux/aur/disktree/PKGBUILD)).
- Arch's own `zed` package and `hummingbird-player` use `-ffat-lto-objects` instead ([zed PKGBUILD](https://gitlab.archlinux.org/archlinux/packaging/packages/zed/-/raw/main/PKGBUILD), [hummingbird-player](https://raw.githubusercontent.com/archlinux/aur/hummingbird-player/PKGBUILD)).

**Precedents**
- Arch `extra/zed` 1.22.0-1 builds from source with cargo.
  - Runtime deps include `vulkan-icd-loader`, `vulkan-driver`, `wayland`, `libxkbcommon(-x11)`, `fontconfig`, `glib2`, `alsa-lib`, `git` ([PKGBUILD](https://gitlab.archlinux.org/archlinux/packaging/packages/zed/-/raw/main/PKGBUILD)).
  - Much of that list is Zed-specific (sqlite, nodejs, alsa).
- `gitcomet-bin` (a gpui git client): `depends=('glibc' 'libgcc' 'git' 'zlib' 'libxcb' 'libxkbcommon' 'libxkbcommon-x11' 'wayland' 'hicolor-icon-theme')`, with `vulkan-icd-loader` optional ([AUR](https://aur.archlinux.org/packages/gitcomet-bin)).
  - Read through a summariser, not byte-verified.
  - Do not copy its deprecated `AGPL-3.0` licence id.
- About 28 gpui apps are on the AUR ([RPC search](https://aur.archlinux.org/rpc/v5/search/gpui?by=name-desc)).

**gdv runtime deps (inferred from the gpui crates at v1.22.0)**
- The Linux renderer is wgpu 29 over Vulkan ([gpui_linux/Cargo.toml](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_linux/Cargo.toml#L18-L63)), so it needs `vulkan-icd-loader` and a driver.
- Wayland is `dlopen`ed (L95-99), and xkbcommon is linked.
- On Omarchy, `amdvlk` is a known breakage ([Zed linux.md](https://github.com/zed-industries/zed/blob/v1.22.0/docs/src/linux.md#L142-L166)).

## 3. `cargo install`

- `cargo install --git <url> [--tag|--rev]` and `--path .` are supported. "By default, `Cargo.lock` is ignored"; pass `--locked` ([cargo-install](https://doc.rust-lang.org/cargo/commands/cargo-install.html)).
- Git dependencies in our Cargo.toml are fine for `cargo install`. They only block `cargo publish`: "crates.io does not allow packages to be published with dependencies on code published outside of crates.io" ([reference](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#multiple-locations)).
  - The `{ git, version }` dual spec would need a crates.io gpui that matches.
  - There isn't one: `gpui` is 0.2.2 from 2025-10-22, and `gpui_platform` is unpublished ([crates.io](https://crates.io/api/v1/crates/gpui), [gpui_platform/Cargo.toml](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_platform/Cargo.toml#L37-L45)).
  - A third-party republish, `gpui-pre` 0.3.7, exists and Zedis uses it ([Zedis](https://github.com/vicanso/zedis)). It is not first-party.
- User-side prerequisites:
  - Rust ≥ Zed's pinned 1.98.1 ([rust-toolchain.toml](https://github.com/zed-industries/zed/blob/v1.22.0/rust-toolchain.toml)). That this is the minimum is inferred.
  - Linux dev headers ([script/linux](https://github.com/zed-industries/zed/blob/v1.22.0/script/linux#L195-L224)).
  - On macOS, full Xcode (§5).

## 4. Flatpak, AppImage, Snap

**Flatpak: reject**
- `/tmp` is a reserved path. It is "not available with `home, host, host-os, host-etc` by default" and must be requested explicitly ([sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html#reserved-paths)).
  - `git difftool` writes its temp files under `$TMPDIR` (normally `/tmp`) and deletes them when the tool returns (git-access research).
  - So a sandboxed gdv can't see them without `--filesystem=/tmp`.
- Running the user's `git` needs `flatpak-spawn --host`, which "requires access to the org.freedesktop.Flatpak D-Bus interface" ([flatpak-spawn(1)](https://man.archlinux.org/man/extra/flatpak/flatpak-spawn.1.en)). That is a full sandbox escape.
  - The alternative, bundling git, loses the user's git version and helpers.
- Flathub: "Static permissions must be kept to an absolute minimum" ([requirements](https://docs.flathub.org/docs/for-app-authors/requirements#permissions)). Host/home filesystem access and `org.freedesktop.Flatpak` are "Granted on sufficient explanation" ([linter](https://docs.flathub.org/docs/for-app-authors/linter)).
- Precedent: Flathub's `dev.zed.Zed` is a community repack. It has `--filesystem=home` plus `--talk-name=org.freedesktop.Flatpak`, and "exits the sandbox on startup" ([manifest](https://github.com/flathub/dev.zed.Zed/blob/1074d0340a669e4babc50d23cd72b4d235b98153/dev.zed.Zed.yaml), [README](https://github.com/flathub/dev.zed.Zed/blob/1074d0340a669e4babc50d23cd72b4d235b98153/README.md)).
- CLI use: Arch adds `/var/lib/flatpak/exports/bin` to `PATH` ([ArchWiki](https://wiki.archlinux.org/title/Flatpak#Run_applications)), but the command is named after the app id, and `flatpak run` filters `PATH` ([flatpak-run(1)](https://man.archlinux.org/man/flatpak-run.1.en)).

**AppImage: works, low value**
- No sandbox, so it sees the host `git` and `/tmp` (inferred from the format).
- It needs FUSE, or `--appimage-extract-and-run`, which "waits until the app closes" ([docs](https://docs.appimage.org/user-guide/troubleshooting/fuse.html#extract-and-run-type-2-appimages)). The static type2-runtime removes the libfuse2 dependency ([type2-runtime](https://github.com/AppImage/type2-runtime)). Whether `fusermount` is still needed is UNVERIFIED.
- Build on the oldest base you support ([best practices](https://docs.appimage.org/reference/best-practices.html)).
- The excludelist does not exclude libvulkan ([excludelist](https://raw.githubusercontent.com/AppImageCommunity/pkg2appimage/master/excludelist)). Whether a bundled Vulkan loader breaks host ICDs is UNVERIFIED.
- GitComet, JayJay and Zedis all ship AppImages (§5).

**Snap: reject**
- Strict confinement's `home` interface covers "non-hidden files" only, so no `~/.gitconfig` ([home interface](https://snapcraft.io/docs/reference/interfaces/home-interface/)).
- Classic confinement needs human store review. The Unsupported list includes "dependent software only available on host" and "access to arbitrary files" ([classic review](https://snapcraft.io/docs/reference/administration/reviewing-classic-confinement-snaps/)).
- On Arch, snapd itself is AUR-only ([snapcraft](https://snapcraft.io/docs/tutorials/install-the-daemon/arch-linux/)).

**Nix (later)**
- `buildRustPackage` with git deps needs `cargoLock.outputHashes` per git source. `allowBuiltinFetchGit` is only for use outside nixpkgs ([nixpkgs manual](https://github.com/NixOS/nixpkgs/blob/master/doc/languages-frameworks/rust.section.md)).

## 5. GitHub Releases and CI

**gpui crate layout at v1.22.0**
- `gpui` is the core. `gpui_platform` pulls in `gpui_macos` or `gpui_linux` ([gpui_platform/Cargo.toml L37-45](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_platform/Cargo.toml#L37-L45)).
- No platform features are on by default. gdv must enable `wayland` (and optionally `x11`) itself ([L16-24](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_platform/Cargo.toml#L16-L24)).
- JayJay and rgitui use the same git-dependency setup.

**Metal shaders: full Xcode needed by default**
- `gpui_apple/build.rs` runs `xcrun -sdk macosx metal` then `metallib` ([L125-179](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_apple/build.rs#L125-L179)). The result is embedded with `include_bytes!` ([metal_renderer.rs L33-34](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_apple/src/metal_renderer.rs#L33-L34)).
- The Command Line Tools alone fail with `unable to find utility "metal"`. On Xcode 26 the Metal toolchain is a separate download: `xcodebuild -downloadComponent MetalToolchain` ([Zed macos.md](https://github.com/zed-industries/zed/blob/v1.22.0/docs/src/development/macos.md#L145-L156)).
- The `runtime_shaders` feature chain (`gpui_platform` → `gpui_macos` → `gpui_apple`) embeds the source instead. The shaders then compile at startup with `new_library_with_source` ([build.rs L20-23](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_apple/build.rs#L20-L23), [metal_renderer.rs L217-223](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_apple/src/metal_renderer.rs#L217-L223)). Zed's releases don't use it.
- cbindgen (pure Rust) generates `scene.h`. A `bindgen` build-dep is declared but unused, so libclang is not needed (inferred) ([gpui/Cargo.toml L162-168](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui/Cargo.toml#L162-L168)).
- The Metal compile has no `-arch` flag ([build.rs L141-150](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_apple/build.rs#L141-L150)). Cross-compiling x86_64 on an arm64 runner has no gpui-specific blocker (inferred).

**Runners** ([runner-images README](https://github.com/actions/runner-images/blob/57b93f2cf7eda14a6a71f3175d15218626cdbd4c/README.md#L24-L36))

| Need | Label | Notes |
|---|---|---|
| macOS arm64 | `macos-15` | default Xcode 16.4 ([readme](https://github.com/actions/runner-images/blob/57b93f2cf7eda14a6a71f3175d15218626cdbd4c/images/macos/macos-15-arm64-Readme.md#L154-L161)) |
| macOS arm64 | `macos-26` / `macos-latest` | Xcode 26.6. The Metal toolchain is installed by the image script ([Xcode.Installer.psm1 L114-117](https://github.com/actions/runner-images/blob/57b93f2cf7eda14a6a71f3175d15218626cdbd4c/images/macos/scripts/helpers/Xcode.Installer.psm1#L114-L117)) but reported missing at times ([#13014](https://github.com/actions/runner-images/issues/13014), [#14013](https://github.com/actions/runner-images/issues/14013), [#14717](https://github.com/actions/runner-images/issues/14717)). |
| macOS x86_64 | `macos-15-intel`, `macos-26-intel` | `macos-13` was retired 2025-12-04. x86_64 support was announced to end with the macOS 15 image in "Fall 2027" ([changelog](https://github.blog/changelog/2025-09-19-github-actions-macos-13-runner-image-is-closing-down/)), but `macos-26-intel` shipped later ([changelog](https://github.blog/changelog/2026-02-26-macos-26-is-now-generally-available-for-github-hosted-runners/)). |
| Linux | `ubuntu-22.04`, `ubuntu-24.04` | — |
| Linux arm64 | `ubuntu-22.04-arm`, `ubuntu-24.04-arm` | free for public repos only ([changelog](https://github.blog/changelog/2025-08-07-arm64-hosted-runners-for-public-repositories-are-now-generally-available/)) |

**glibc baseline**
- Zed requires glibc ≥ 2.31 on x86_64 and ≥ 2.35 on aarch64 ([linux.md L24-27](https://github.com/zed-industries/zed/blob/v1.22.0/docs/src/linux.md#L24-L27)).
- JayJay builds inside an `ubuntu:20.04` container (glibc 2.31) ([appimage.yml](https://github.com/hewigovens/jayjay/blob/eb86b0b6646ac53a8beeb8458133f8c7fd8b4a0a/.github/workflows/appimage.yml#L33-L86)).
- Building on the `ubuntu-22.04` runners gives glibc 2.35. That is fine for Arch, and that it is fine for any distro still in support is inferred.

**What other gpui apps do**

| App | macOS artifacts | Signed / notarized | Other channels |
|---|---|---|---|
| Zed | Separate arm64 and x86_64 DMGs, no `lipo` ([bundle-mac L79-94, L250-264](https://github.com/zed-industries/zed/blob/v1.22.0/script/bundle-mac#L79-L94)). `.app` via a `cargo-bundle` fork. CLI at `Contents/MacOS/cli`. | Yes: `codesign --options runtime --timestamp`, `notarytool submit --wait` with an App Store Connect API key, `stapler staple` on the DMG ([L207-211, L276-281](https://github.com/zed-industries/zed/blob/v1.22.0/script/bundle-mac#L207-L211)). Falls back to ad-hoc without secrets. | Linux tarball: `bin/zed` (cli) plus `libexec/zed-editor` ([bundle-linux L145-212](https://github.com/zed-industries/zed/blob/v1.22.0/script/bundle-linux#L145-L212)), installed by `install.sh` ([L115-168](https://github.com/zed-industries/zed/blob/v1.22.0/script/install.sh#L115-L137)). Release jobs run on Namespace runners, not GitHub's ([release.yml](https://github.com/zed-industries/zed/blob/v1.22.0/.github/workflows/release.yml)). |
| GitComet | Per-arch DMGs | Yes, when secrets are set ([build-release-artifacts.yml](https://github.com/Auto-Explore/GitComet/blob/9348a552fed5cba01650e3aa1650f9019478bc92/.github/workflows/build-release-artifacts.yml#L616-L879)) | AppImage, tarball, `.deb`, own tap cask, AUR `-bin` |
| Zedis | Per-arch DMGs via `cargo-bundle` | Yes, with `notarytool` ([publish.yml](https://github.com/vicanso/zedis/blob/747d4445a747f9f8e1159b46492a1c25033abd28/.github/workflows/publish.yml#L153-L291)) | Official cask, AUR `-bin`, AppImage/`.deb`/`.rpm` |
| rgitui | arm64-only DMG | Ad-hoc only ([bundle-mac.sh L99-101](https://github.com/noahbclarkson/rgitui/blob/ead56a9ec9d43cf099f27ae1129323a08d56149e/script/bundle-mac.sh#L99-L101)) | — |
| Loungy (inactive) | Universal binary: `lipo -create` of both targets built on one arm64 runner ([release-macos.yml L13-33](https://github.com/MatthiasGrandl/Loungy/blob/303e7d2c46db13c49f8c7ca30762e5ee0ac582d2/.github/workflows/release-macos.yml#L13-L33)) | No | — |
| JayJay | macOS app is SwiftUI, not gpui | — | gpui on Linux only: AppImage plus a signed pacman repo ([README](https://github.com/hewigovens/jayjay/blob/eb86b0b6646ac53a8beeb8458133f8c7fd8b4a0a/README.md)) |

**cargo-dist (`dist`)**
- Maintained: v0.33.0 was released 2026-09-11 ([releases](https://github.com/axodotdev/cargo-dist/releases)).
- It produces tarballs, a shell installer, and a tap **formula** with per-arch URLs ([homebrew.md](https://github.com/axodotdev/cargo-dist/blob/v0.33.0/book/src/installers/homebrew.md)).
- It has no `.app`/DMG and no notarization; `codesign` only ([sign/macos.rs](https://github.com/axodotdev/cargo-dist/blob/v0.33.0/cargo-dist/src/sign/macos.rs#L1-L21), [#1534](https://github.com/axodotdev/cargo-dist/issues/1534)).
- That gap doesn't matter for the unsigned-binary MVP, and dist fits it.
- Git-dependency support is UNVERIFIED. It runs plain `cargo build`, so git deps are expected to work.

## 6. macOS signing, notarization, Gatekeeper, `.app`

**Cost and tools**
- The Apple Developer Program is "99 USD per membership year" ([enroll](https://developer.apple.com/programs/enroll/)).
- Use the Developer ID Application certificate for apps and command-line tools ([notarization issues](https://developer.apple.com/documentation/security/resolving-common-notarization-issues#Use-a-valid-Developer-ID-certificate), [Developer ID](https://developer.apple.com/developer-id/)).
- Since 2023-11-01 the notary service accepts only `notarytool`. Requirements: hardened runtime for app and command-line targets, a secure timestamp, and Developer ID ([notarizing](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution), [hardened runtime](https://developer.apple.com/documentation/security/hardened-runtime)).
- Stapling: "Although tickets are created for standalone binaries, it's not currently possible to staple tickets to them" ([customizing the workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow#Staple-the-ticket-to-your-distribution)). Staple the outer DMG or pkg instead ([DTS](https://developer.apple.com/forums/thread/689337)).

**Quarantine and Gatekeeper behaviour**

| How the binary arrives | Quarantined? | Result for an unsigned or ad-hoc binary |
|---|---|---|
| Browser download, extracted with Archive Utility (Finder double-click or Safari auto-extract), run from Terminal | Yes. Safari sets `com.apple.quarantine`, and Archive Utility propagates it ([DTS](https://developer.apple.com/forums/thread/706442)). | "If you launch a quarantined app, the system invokes Gatekeeper" (same thread). Ad-hoc signatures "cannot pass through Gatekeeper" ([Big Sur notes](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes)). So it is blocked until approved in Privacy & Security → "Open Anyway" ([support](https://support.apple.com/en-us/102445)). Since Sequoia there is no Control-click override ([Apple news](https://developer.apple.com/news/?id=saqachfa)). The exact Terminal-side error is UNVERIFIED. |
| Browser download, extracted with `tar xzf` in Terminal | The tarball is quarantined; the extracted binary is not, because `tar` doesn't propagate it ([DTS](https://developer.apple.com/forums/thread/706442); verify) | Runs |
| `curl` download, `tar` extract | No: "Unix-y networking tools, like `curl` and `scp`, don't quarantine"; `tar`/`unzip` "don't propagate quarantine" ([DTS](https://developer.apple.com/forums/thread/706442)) | Runs. On arm64 the linker's automatic ad-hoc signature is enough. |
| Homebrew formula | No (inferred from code, §1) | Runs |
| Homebrew cask | Yes ([docs](https://docs.brew.sh/Homebrew-Security-and-Supply-Chain#casks-have-a-different-trust-model)) | Unsigned casks are disabled in homebrew/cask (§1) |

- Gatekeeper checks software "the first time it is opened" ([Apple Platform Security](https://support.apple.com/guide/security/gatekeeper-and-runtime-protection-sec5599b66df/web)).
- On arm64, re-sign with `codesign -s -` after any `strip` or `install_name_tool` ([Big Sur notes](https://developer.apple.com/documentation/macos-release-notes/macos-big-sur-11_0_1-universal-apps-release-notes)).

**Does gpui need a `.app` bundle?** The core features don't. All references are in [`crates/gpui_macos/src/platform.rs`](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_macos/src/platform.rs) at v1.22.0:
- **Activation policy:** `setActivationPolicy_(NSApplicationActivationPolicyRegular)` in `applicationDidFinishLaunching` (L1320). It is set in code, not from Info.plist.
- **Focus:** the app must call `cx.activate(true)`, which maps to `activateIgnoringOtherApps_` (L625-628). Otherwise the window may stay behind the terminal (inferred).
- **Menu bar:** set in code with `setMainMenu_` (L1084).
- **Unbundled launch is handled:**
  - `app_path` falls back to the executable's directory (L585) and errors "app is not running inside a bundle" (L1071-1073).
  - URL-scheme registration requires a bundle (L774-775). gdv doesn't need it.
- **Retina:** `window.rs` reads `backingScaleFactor` (L2433) and sets `setWantsBestResolutionOpenGLSurface_(YES)` (L1183) ([window.rs](https://github.com/zed-industries/zed/blob/v1.22.0/crates/gpui_macos/src/window.rs)). The default of `NSHighResolutionCapable` for a binary without Info.plist is UNVERIFIED ([Apple](https://developer.apple.com/documentation/bundleresources/information-property-list/nshighresolutioncapable)).
- **Without a bundle, expect** a generic Dock icon and the process name in the menu bar (inferred).
- **How Zed ships its CLI:** a separate `cli` binary inside `Zed.app/Contents/MacOS/`, symlinked as `zed` ([install_cli](https://github.com/zed-industries/zed/blob/v1.22.0/crates/install_cli/src/install_cli_binary.rs#L22-L60), cask `binary` stanza).
  - It talks to the app process. gdv doesn't need that split: one process is the CLI and the window.

## 7. Minimum git version

Method: the first release tag containing the commit that added each option, from a full-history clone of git.git, cross-checked against `Documentation/RelNotes`. RelNotes links are at git.git `a018953`.

| Command / feature gdv uses | First version | Source |
|---|---|---|
| `git diff --raw -z -M` | ≤ 1.6.0 (documented in `diff-options.txt` at v1.6.0) | [`--raw` L36](https://github.com/git/git/blob/v1.6.0/Documentation/diff-options.txt#L36), [`-z` L77](https://github.com/git/git/blob/v1.6.0/Documentation/diff-options.txt#L77), [`-M` L129](https://github.com/git/git/blob/v1.6.0/Documentation/diff-options.txt#L129) |
| `--no-abbrev` | ≤ 1.6.0 (`revision.c` parses it at v1.6.0) | [revision.c L1133](https://github.com/git/git/blob/v1.6.0/revision.c#L1133) |
| `git status --porcelain=v2` (and `-z` with it) | **2.11.0** | commit [1cd828d](https://github.com/git/git/commit/1cd828ddc8a61f6bec93303f4bdbcdbf1c261cb7); [RelNotes 2.11.0 L168](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/RelNotes/2.11.0.adoc?plain=1#L168) |
| `git cat-file --batch` | 1.5.6 | commit [05d5667](https://github.com/git/git/commit/05d5667fec9650b049f47edd8cca23a43b135365) |
| `git cat-file --textconv` (single object) | 1.7.2 | commit [9f77fe0](https://github.com/git/git/commit/9f77fe022425360dced8b1f19275852e89d2351c) |
| `git cat-file --filters` | 2.11.0 | commit [b9e62f6](https://github.com/git/git/commit/b9e62f60115c75c5be5de593862925c8b8d7e683); [RelNotes 2.11.0 L59-66](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/RelNotes/2.11.0.adoc?plain=1#L59-L66) |
| **`--batch` combined with `--textconv`/`--filters`** | **2.11.0** | commit [3214594](https://github.com/git/git/commit/321459439e19517c412cab1cfbb64a2749f272c9) "cat-file: support --textconv/--filters in batch mode" |
| `git blame --incremental` | 1.5.0 | [RelNotes 1.5.0 L347](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/RelNotes/1.5.0.adoc?plain=1#L347); commit [57e7a0a](https://github.com/git/git/commit/57e7a0a49496fa3c2b2514b6a23ad661ecb428a6) |
| `git blame --contents <file>` (against HEAD) | 1.5.0 | commit [06e75a7](https://github.com/git/git/commit/06e75a72372d0466e110002a7504fefebe0e9c52) |
| `git blame --contents <file> <rev>` | 2.41.0, **not needed** | [RelNotes 2.41.0 L41-43](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/RelNotes/2.41.0.adoc?plain=1#L41-L43): "used to be forbidden". The blame design blames working-tree and index Sides with `--contents` against HEAD, and revision Sides with plain `blame <rev>`. |
| `blame.ignoreRevsFile` / `--ignore-revs-file` | **2.23.0** | commit [ae3f36d](https://github.com/git/git/commit/ae3f36dea16e51041c56ba9ed6b38380c8421816); [RelNotes 2.23.0 L83](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/RelNotes/2.23.0.adoc?plain=1#L83) |
| ignore-revs validation fix; tolerating non-existent object names | 2.29.0 / **2.30.0** | [RelNotes 2.29.0 L447](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/RelNotes/2.29.0.adoc?plain=1#L447), [RelNotes 2.30.0 L277-278](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/RelNotes/2.30.0.adoc?plain=1#L277-L278) |
| `git difftool -d` / `--dir-diff` | 1.7.11 (rewritten as a builtin in 2.12.0) | [RelNotes 1.7.11 L51](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/RelNotes/1.7.11.adoc?plain=1#L51); commit [7e0abce](https://github.com/git/git/commit/7e0abcec103b3649943b236881cf88e8fd6cf3a4); builtin [03831ef](https://github.com/git/git/commit/03831ef7b5011cbedbcc67abe5f47762f37c6c21) |
| `cat-file --batch-command` (avoid) | 2.36.0 | [RelNotes 2.36.0 L75](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/RelNotes/2.36.0.adoc?plain=1#L75) |

**What ships today**

| Platform | git version | Source |
|---|---|---|
| Debian 11 | 2.30.2 | [packages.debian.org](https://packages.debian.org/search?keywords=git&searchon=names&exact=1&suite=all&section=all) |
| Debian 12 | 2.39.5 | same |
| Debian 13 | 2.47.3 | same |
| Ubuntu 22.04 | 2.34.1 | [packages.ubuntu.com](https://packages.ubuntu.com/search?keywords=git&searchon=names&exact=1&suite=all&section=all) |
| Ubuntu 24.04 | 2.43.0 | same |
| Ubuntu 26.04 | 2.53.0 | same |
| macOS 15.6 Command Line Tools | 2.39.5 (Apple Git-154) | [Apple forums, Aug 2025](https://developer.apple.com/forums/thread/795617) |

- The git in the current (Xcode 26) Command Line Tools is UNVERIFIED.
- Arch tracks upstream.

**Recommendation:** check `git --version` at startup and refuse to run below **2.11** with a clear message. Document **2.30** as the supported floor.

## 8. macOS `/usr/bin/git` without the Command Line Tools

- `/usr/bin/git` is a shim: "macOS 10.9 and later includes shims or wrapper executables … in `/usr/bin`" that forward to the active developer directory. The Command Line Tools live in `/Library/Developer/CommandLineTools`, and `xcode-select --print-path` shows the active directory ([TN2339](https://developer.apple.com/library/archive/technotes/tn2339/_index.html)).
- With no tools installed, running the shim pops the GUI "install the command line developer tools" dialog and exits non-zero. The exact message and exit code are UNVERIFIED in Apple docs, and Apple's online xcode-select(1)/xcrun(1) man pages could not be fetched.
- After a macOS upgrade the active path can dangle. git then fails with `xcrun: error: invalid active developer path (/Library/Developer/CommandLineTools), missing xcrun` ([Apple forums 673827](https://developer.apple.com/forums/thread/673827), [711512](https://developer.apple.com/forums/thread/711512)).
- **Detecting this without triggering the dialog** (prior art: Homebrew's installer):
  - Check the file `-e /Library/Developer/CommandLineTools/usr/bin/git` ([install.sh L446-453](https://github.com/Homebrew/install/blob/HEAD/install.sh#L446-L453)).
  - Run `/usr/bin/xcode-select --print-path 2>/dev/null` (L926).
  - `brew` itself checks `CommandLineTools/usr/bin/clang` and the `com.apple.pkg.CLTools_Executables` pkgutil receipt ([xcode.rb L248-405](https://github.com/Homebrew/brew/blob/d6ca35509427ed7b4e37445054fa343ab90755ec/Library/Homebrew/os/mac/xcode.rb#L248-L405)).
- **Recommended logic for gdv:**
  1. Resolve `git` on `PATH`. Homebrew's `/opt/homebrew/bin/git` normally comes before `/usr/bin`.
  2. If the result is `/usr/bin/git`, run `xcode-select -p`. If that fails, or `<path>/usr/bin/git` does not exist, print "git not found: run `xcode-select --install` or `brew install git`" and exit 2 **before** spawning `/usr/bin/git`.
  3. Never spawn the shim speculatively: a GUI dialog popping up from a CLI is the failure mode.
  - `xcrun --find git` is itself a shim, so it may trigger the same dialog (UNVERIFIED). Don't use it for detection.
- **Homebrew users may have no Apple git at all:** "On Apple Silicon, casks and bottles can be installed without developer tools" ([docs](https://docs.brew.sh/Installation#3)). The tap formula should declare `depends_on "git"`, which guarantees a working git on `PATH`.

## Risks

1. **Unbundled gpui window on macOS is untested.** Add these to the macOS gpui check (ticket 09):
   - run the bare release binary from Terminal;
   - check that the window takes focus (`cx.activate(true)`), Retina scale, Dock icon and name;
   - check that `cmd-q` and window close return control to the shell with exit 0.
2. **Two quarantine facts rest on inference or a forum post,** not docs: formulae are not quarantined, and `tar` does not propagate quarantine. Verify both on a Mac with `xattr -l $(which gdv)` after a tap install and after a `curl | tar` install.
3. **The Metal toolchain on Xcode 26 runners is flaky.** Mitigate with the explicit download step, pinning `macos-15`, or the `runtime_shaders` fallback, whose startup cost is unmeasured.
4. **The Intel Mac tail is shrinking.** Homebrew 7.0.0 made Intel Tier 3, and GitHub's x86_64 macOS runners have an announced but now unclear end date. Keep the x86_64 build while it is free; drop it when runners go.
5. **Source builds clone all of Zed** (AUR source package, `cargo install --git`, a source formula). This is slow and large. Shallow git deps are unstable in cargo.
6. **A tap formula that installs a prebuilt GUI binary has no gpui precedent.** Every gpui app on Homebrew uses a signed cask. Homebrew could tighten third-party tap rules; Tap Trust in 6.0.0 is the latest change.
7. **Homebrew docs were heavily rewritten in 2026.** Policy pages carry review dates from 2026-06 to 2026-09. Re-check before a homebrew/core submission.
8. **Linux runtime:** gpui needs a working Vulkan driver (wgpu). Failures show as `NoSupportedDeviceFound`, and `amdvlk` breaks on Omarchy ([Zed linux.md](https://github.com/zed-industries/zed/blob/v1.22.0/docs/src/linux.md#L142-L166)). This is a support burden, not a packaging blocker.
9. **The gdv repo has no `Cargo.lock` yet.** `--locked`/`--frozen` (AUR, Homebrew `fetch`, `cargo install --locked`) need it committed.
