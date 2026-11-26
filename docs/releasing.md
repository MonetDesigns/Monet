# Releasing Monet

> **Status: desktop packaging migrated; dry run pending.** `packaging/` now builds `-p monet` and
> every desktop artifact is named `monet`. The web front-end, headless CLI and collaboration server
> are still the upstream `figcraft-*` crates, so their release channels are switched off in
> `.github/workflows/release.yml` until each carries its own Monet name. Automatic releases stay off
> until one `workflow_dispatch` dry run produces a draft that installs and starts.

`workflow_dispatch` builds the unsigned desktop channels (Linux, Flatpak, Windows) without creating
a release; macOS and the draft job need the `release` environment and are skipped off that branch.
A manual run is a dry run until the checklist below passes.

## Monet's names

| Thing | Value |
|---|---|
| Display name | Monet |
| Desktop package / binary | `monet` |
| Bundle / app id | `app.monet.design` |
| Native document | `.figcraft`, MIME `application/x-figcraft`, UTI `app.monet.design.document` |
| Also opens | `.svg`, `.png`, `.jpg`/`.jpeg` ("Open with", never the default app) |
| WiX UpgradeCode | `7F8B9827-D065-41B7-A514-2A454B21F484` (fixed forever; never reuse it in another app) |
| Linux categories | `Graphics;2DGraphics;VectorGraphics;X-Design;` (`Design` isn't a registered freedesktop category, so `desktop-file-validate` rejects it) |

Legacy names that are **not** release targets yet:

| Thing | Current name | Why it is off |
|---|---|---|
| Headless CLI | `figcraft-cli` | its clap name, help, man pages and completion files still say figcraft |
| Collaboration server | `figcraft-server` | its crate, service units, install paths and docs still say figcraft |
| Browser build | `figcraft-web` | no Monet web front-end exists; the tree is the port reference |

Do not stage any of them into a Monet installer until the rename lands: `packaging/env.sh` builds
only `monet`, and `release.yml` sets `has-web`, `has-server` and `has-freebsd` to false.

## Cutting a release

1. **Bump the version** on `main`. It lives only in `[workspace.package] version` in the root
   `Cargo.toml`:

   ```sh
   cargo run -p xtask -- version              # prints the current version, e.g. 2648.15.0
   cargo run -p xtask -- version set 2648.16.0 # or 2648.16.0-rc.1; updates Cargo.toml and Cargo.lock
   ```

   Commit `Cargo.toml` + `Cargo.lock`.
2. **Run the workflow manually** until automatic releases are re-enabled:
   `gh workflow run release.yml --ref <branch> -f version=2648.16.0-rc.1`.
3. **Check the draft** (`Monet v2648.16.0`, tagged `v2648.16.0` on the pushed commit) with every artifact
   and `SHA256SUMS.txt`. Warnings in the job summaries mean a signing secret was missing.
4. **Install and start each package** before publishing: the AppImage/deb/rpm on Linux, the MSI and
   portable zip on Windows, the DMG on macOS. Launch the app, then check `monet --version` exits 0
   (release Windows builds have no console output; the About window shows the same version).
5. **Publish** the draft, then re-enable `push: branches: [release]` in `release.yml`.

Pushing to `release` again before publishing rebuilds the same draft and replaces its assets.
After the draft is published, the workflow refuses to touch that version again, so bump first.

## Build variables

`packaging/env.sh` reads `MONET_VERSION`, `MONET_BUILD_SHA` and `MONET_BUILD_DATE`, with the old
`FIGCRAFT_*` names accepted as fallbacks for one transition. The shared release action exports the
`MONET_*` names because `app-name` is `monet`.

The engine and document crates still read their upstream build variables:

```sh
FIGCRAFT_DEFAULT_SERVER= cargo build --release -p monet          # no default server
FIGCRAFT_DEFAULT_SERVER=wss://design.example.org/ cargo build --release -p monet
FIGCRAFT_DEFAULT_FONT="Noto Sans Thai" cargo build --release -p monet
```

The release workflow leaves them unset.

## What gets built

| Platform | Artifact | Built on |
|---|---|---|
| macOS 11+ (universal) | `monet-<v>-macos-universal.dmg` | `macos-15-xlarge` |
| Windows 10+ x64 / x86 / arm64 | `monet-<v>-windows-<arch>.msi`, `monet-<v>-windows-<arch>-portable.zip` | `windows-latest` (arm64 cross-compiled) |
| Linux x86_64 / aarch64 | `monet-<v>-linux-<arch>.{AppImage,AppImage.zsync,deb,rpm,tar.gz,flatpak}` | `ubuntu-22.04` / `ubuntu-22.04-arm` (Flatpak: 24.04) |
| Linux riscv64 | `monet-<v>-linux-riscv64.tar.gz` (needs glibc ≥ 2.39) | cross-compiled on `ubuntu-24.04` against ports.ubuntu.com riscv64 libraries |

`monet --version` exits 0 on every platform (Windows release builds are GUI-subsystem, so they
print nothing to a console; use a debug `cargo run -p monet -- --version` to read the string).
Desktop packages ship no `figcraft-cli`.

**Fonts.** Every build job checks out the `craft-fonts` build input at `craft-fonts-ref` and builds
with `CRAFT_FONTS_DIR` and `CRAFT_FONTS_REQUIRED=1`, so releases embed its fonts and fail rather
than ship without them. `crates/text/build.rs` reads both `fonts/manifest.txt` and
`fonts/latin-manifest.txt`, so the pin must combine the Latin presentation catalog and the current
core UI faces. Packages carry each embedded font's licence as `OFL-<family>.txt`
(`copy_font_licences` in `packaging/env.sh`; `Contents/Resources/Licenses` in the macOS app).

### macOS

`packaging/macos/package.sh [--arch universal|aarch64|x86_64]` builds both Apple targets with
`MACOSX_DEPLOYMENT_TARGET=11.0`, joins them with `lipo` and assembles `Monet.app`
(`Contents/MacOS/Monet`, `Contents/Resources/Monet.icns`, `Info.plist` from `Info.plist.in`).
The `.icns` is generated at packaging time with `iconutil` from the committed
`assets/app-icon/monet-<size>.png` (made by `cargo xtask icon`); it is never committed.
Signing goes inside-out with the hardened runtime and a timestamp (empty entitlements), then the
app is notarized and stapled, put on a DMG with an `Applications` link (`hdiutil makehybrid` +
`convert`), and the DMG is signed, notarized and stapled. `packaging/macos/verify.sh` checks the
artifact as shipped (strict with secrets, warnings without).

Locally (no certificates: ad-hoc signature, no notarization):

```sh
packaging/macos/package.sh --arch aarch64     # quick, host-only; omit --arch for universal
packaging/macos/verify.sh --arch aarch64
hdiutil attach -nobrowse -readonly dist/release/monet-*-macos-aarch64.dmg   # inspect, then detach
```

### Windows

`packaging/windows/package.ps1 -Arch x64|x86|arm64` builds with `+crt-static` (no VC++
redistributable), checks the exe's PE machine and GUI subsystem, signs it, builds the MSI with WiX
v5 (`monet.wxs`: per-user default with optional machine scope, Start Menu shortcut, owner of
`.figcraft`, "Open with" for SVG/PNG/JPEG, App Paths, setup wizard with art drawn from the app
icon), validates ICE50, signs the MSI and zips a portable build. `sign.ps1` uses a `.pfx` (`WINDOWS_CERTIFICATE*`) or Azure Trusted
Signing (`AZURE_*`), else warns and leaves files unsigned. `.github/workflows/windows-arm64.yml`
installs the arm64 MSI on real ARM64 hardware nightly.

Locally on Windows: `dotnet tool install -g wix --version 5.0.2`,
`wix extension add -g WixToolset.UI.wixext/5.0.2 WixToolset.Util.wixext/5.0.2`, then
`pwsh packaging/windows/package.ps1 -Arch x64`.

### Linux

`packaging/linux/package.sh [--formats "appimage deb rpm tar"]` stages one FHS tree (the `monet`
binary, `app.monet.design.desktop`, hicolor icons 16–512 px, AppStream metainfo, a shared-mime-info
file for `.figcraft`) and builds every format from it: `.tar.gz`; `.deb` and `.rpm` with
[nfpm](https://nfpm.goreleaser.com/) (`nfpm.yaml` lists the dlopen'd runtime libraries as
dependencies); and an `.AppImage` with update information and a `.zsync`, whose `AppRun` registers
the desktop entry and icons on launch so Wayland docks show the right icon
(`MONET_NO_DESKTOP_INTEGRATION=1` opts out; covered by `apprun-test.sh`).
`flatpak-bundle.sh` repackages the tarball into a `.flatpak` with
`flatpak/app.monet.design.bundle.yml` (freedesktop 26.08). `flatpak/app.monet.design.yml` is the
from-source manifest for a Flathub submission (not built in CI; vendor crates with
`flatpak-cargo-generator.py` first). Both manifests keep the same runtime and `finish-args`
(packaging-lint checks). Binaries are built on Ubuntu 22.04 and need glibc ≥ 2.35. The
`linux-riscv64` job cross-compiles with `CROSS_ARCH`/`CROSS_TARGET`/`CROSS_COMPILE` and
`EMULATOR=qemu-riscv64` and packages only the tarball (`--formats tar`).

### FreeBSD, web and server

These channels are disabled in `release.yml` and their packaging still carries upstream names:

- `packaging/freebsd/package.sh` now stages `monet` and passes `--dry-run`, but the shared FreeBSD
  action expects `ai.storyteller.monet.desktop`; keep `has-freebsd: false` until the action or the
  desktop file naming is reconciled.
- `packaging/web/package.sh` builds `apps/figcraft-web`; there is no Monet web front-end.
- `packaging/server/package.sh` builds `figcraft-server`; see [`server.md`](server.md).

## Secrets

All signing secrets are **`release`-environment secrets** (deployment branches: `release` only),
never repo or org secrets. Names (identical in every app): `APPLE_CERTIFICATE`,
`APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`,
`WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`, `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`,
`AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE`.
Each is optional: a missing one gives an unsigned artifact and a `::warning::`. An org admin
provisions the environment, ruleset and secrets, and adds the repo to the org's Default runner
group so `macos-15-xlarge` can pick up the job.

## Checks

- `.github/workflows/ci.yml` (PRs and `main`, Linux only): fmt, clippy `-D warnings`, tests,
  layering, asset attribution, wasm32 check. No macOS in CI (playbook).
- `.github/workflows/packaging-lint.yml` (changes to `packaging/` or workflows): actionlint,
  shellcheck, the AppRun test, a PowerShell parse, the MSI icon check, the FreeBSD dry run, a
  grep for other products' names, xmllint, `desktop-file-validate`, `appstreamcli validate` and
  the Flatpak manifest agreement check.
