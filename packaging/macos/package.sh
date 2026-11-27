#!/usr/bin/env bash
# Build, sign and (optionally) notarize the macOS release artifact:
#
#   $DIST/monet-<version>-macos-<arch>.dmg          Monet.app on a drag-to-Applications DMG
#
# The desktop package ships the app only; the headless CLI is not staged until it carries its
# own Monet name (docs/development-model.md).
#
# Usage: packaging/macos/package.sh [--arch universal|aarch64|x86_64] [--skip-build]
#        [--require-signing]
#
# Signing (env):
#   MACOS_SIGN_IDENTITY   codesign identity (name or SHA-1). Default "-" = ad-hoc (local testing;
#                         Gatekeeper will reject the result on other Macs). In CI, import-cert.sh sets it.
#   MACOS_KEYCHAIN        keychain holding the identity (optional)
# Notarization (env; all three needed, and a real identity):
#   APPLE_ID, APPLE_PASSWORD (app-specific password), APPLE_TEAM_ID
#   --require-signing     fail instead of warning when the identity or notarization secrets are
#                         missing; release builds set MONET_REQUIRE_SIGNING=1 too.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/macos"

ARCH=universal
SKIP_BUILD=0
REQUIRE_SIGNING="${MONET_REQUIRE_SIGNING:-0}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --require-signing) REQUIRE_SIGNING=1; shift ;;
    -h | --help) sed -n '2,18p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  aarch64) TARGETS=(aarch64-apple-darwin) ;;
  x86_64) TARGETS=(x86_64-apple-darwin) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

# Keep in sync with LSMinimumSystemVersion in Info.plist.in.
export MACOSX_DEPLOYMENT_TARGET=11.0
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
SHORT_VERSION="${VERSION%%-*}"
WORK="$CARGO_TARGET_DIR/macos-package"
APP="$WORK/Monet.app"
DMG="$DIST/monet-$VERSION-macos-$ARCH.dmg"

NOTARIZE=0
if [[ "$REQUIRE_SIGNING" = 1 ]]; then
  if [[ "$IDENTITY" = "-" ]]; then
    echo "error: --require-signing needs MACOS_SIGN_IDENTITY (ad-hoc builds are not releasable)" >&2
    exit 2
  fi
  if [[ -z "${APPLE_ID:-}" || -z "${APPLE_PASSWORD:-}" || -z "${APPLE_TEAM_ID:-}" ]]; then
    echo "error: --require-signing needs APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID" >&2
    exit 2
  fi
  if grep -q '^| `assets/devices/`.*UNCONFIRMED' "$ROOT/ATTRIBUTION.md"; then
    echo "error: assets/devices has no recorded redistribution licence; update ATTRIBUTION.md before a release build" >&2
    exit 2
  fi
fi
if [[ "$IDENTITY" = "-" ]]; then
  warn "macOS: ad-hoc signing (no MACOS_SIGN_IDENTITY); artifacts are not notarized"
elif [[ -n "${APPLE_ID:-}" ]] && [[ -n "${APPLE_PASSWORD:-}" ]] && [[ -n "${APPLE_TEAM_ID:-}" ]]; then
  NOTARIZE=1
else
  warn "macOS: APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID incomplete; signed but not notarized"
fi

echo "==> Monet $VERSION for macOS ($ARCH; ${BINS[*]}), identity: $IDENTITY, notarize: $NOTARIZE"

# ---- build -------------------------------------------------------------------------------------
if [[ "$SKIP_BUILD" = 0 ]]; then
  args=()
  for t in "${TARGETS[@]}"; do args+=(--target "$t"); done
  (cd "$ROOT" && cargo build --release --locked "${CARGO_PKGS[@]}" "${args[@]}")
fi

rm -rf "$WORK"
mkdir -p "$WORK/bin"
for bin in "${BINS[@]}"; do
  inputs=()
  for t in "${TARGETS[@]}"; do inputs+=("$CARGO_TARGET_DIR/$t/release/$bin"); done
  lipo -create -output "$WORK/bin/$bin" "${inputs[@]}"
  lipo -info "$WORK/bin/$bin"
done

# ---- signing helpers ---------------------------------------------------------------------------
sign() {
  # Hardened runtime + secure timestamp for a real identity; ad-hoc can't be timestamped.
  local ts=(--timestamp)
  [[ "$IDENTITY" = "-" ]] && ts=(--timestamp=none)
  local kc=()
  [[ -n "${MACOS_KEYCHAIN:-}" ]] && kc=(--keychain "$MACOS_KEYCHAIN")
  codesign --force --sign "$IDENTITY" ${kc[@]+"${kc[@]}"} "${ts[@]}" "$@"
}

notarize() {
  local file="$1" out id status
  echo "==> notarizing $(basename "$file") (this can take a few minutes)"
  out="$(xcrun notarytool submit "$file" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" --wait --timeout 1h --output-format json)" || true
  echo "$out"
  id="$(printf '%s' "$out" | plutil -extract id raw -o - - 2>/dev/null || true)"
  status="$(printf '%s' "$out" | plutil -extract status raw -o - - 2>/dev/null || true)"
  if [[ "$status" != "Accepted" ]]; then
    if [[ -n "$id" ]]; then
      xcrun notarytool log "$id" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" || true
    fi
    echo "error: notarization of $(basename "$file") failed (status: ${status:-unknown})" >&2
    exit 1
  fi
}

# ---- icon --------------------------------------------------------------------------------------
# The .icns is generated here from the committed PNGs (assets/app-icon/monet-<size>.png, made by
# `cargo xtask icon`); it is never committed.
ICONS="$ROOT/assets/app-icon"
ICONSET="$WORK/Monet.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  cp "$ICONS/monet-$s.png" "$ICONSET/icon_${s}x${s}.png"
  cp "$ICONS/monet-$((s * 2)).png" "$ICONSET/icon_${s}x${s}@2x.png"
done
iconutil -c icns -o "$WORK/Monet.icns" "$ICONSET"

# ---- Monet.app ---------------------------------------------------------------------------------
echo "==> assembling $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# Executable and icon carry the display name (CFBundleExecutable / CFBundleIconFile).
cp "$WORK/bin/monet" "$APP/Contents/MacOS/Monet"
cp "$WORK/Monet.icns" "$APP/Contents/Resources/Monet.icns"
# Runtime assets are external on purpose: the device catalogue, factory brush and default WGSL
# material are read from files, not compiled into the binary. They must live at the standard
# Contents/Resources path, which crates/devices, crates/abr and ui-gpui also look for.
mkdir -p "$APP/Contents/Resources/assets"
rm -rf "$APP/Contents/Resources/assets/devices" "$APP/Contents/Resources/assets/brushes" "$APP/Contents/Resources/assets/wgsl"
cp -R "$ROOT/assets/devices" "$APP/Contents/Resources/assets/devices"
cp -R "$ROOT/assets/brushes" "$APP/Contents/Resources/assets/brushes"
cp -R "$ROOT/assets/wgsl" "$APP/Contents/Resources/assets/wgsl"
cp "$ROOT/NOTICE" "$APP/Contents/Resources/"
# NOTICE points at ATTRIBUTION.md for third-party assets; ship the table with it.
cp "$ROOT/ATTRIBUTION.md" "$APP/Contents/Resources/"
# Licences of the embedded craft-fonts fonts (only when built with CRAFT_FONTS_DIR).
if [[ -n "${CRAFT_FONTS_DIR:-}" ]]; then
  mkdir -p "$APP/Contents/Resources/Licenses"
  copy_font_licences "$APP/Contents/Resources/Licenses"
fi
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${MONET_BUILD_SHA:-unknown}/g" \
  "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

# Sign inside-out: nested code first, then the bundle itself (no --deep on the final signature).
# Today the only nested code is the main executable; frameworks/helpers would be signed here too.
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP/Contents/MacOS/Monet"
sign --options runtime --entitlements "$HERE/entitlements.plist" "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

if [[ "$NOTARIZE" = 1 ]]; then
  ditto -c -k --keepParent "$APP" "$WORK/Monet-notarize.zip"
  notarize "$WORK/Monet-notarize.zip"
  retry xcrun stapler staple "$APP"
  retry xcrun stapler validate "$APP"
  spctl --assess --type execute -vvv "$APP"
fi

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> building $DMG"
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/Monet.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG" "$WORK/raw.dmg"
# makehybrid + convert builds the image without attaching a device, unlike `create -srcfolder`,
# which is flaky on CI runners ("Resource busy") and hangs in sandboxed sessions.
hdiutil makehybrid -hfs -hfs-volume-name "Monet" -hfs-openfolder "$STAGE" -o "$WORK/raw.dmg" "$STAGE"
hdiutil convert "$WORK/raw.dmg" -format UDZO -imagekey zlib-level=9 -o "$DMG"
rm -f "$WORK/raw.dmg"
sign "$DMG"
codesign --verify --strict --verbose=2 "$DMG"
if [[ "$NOTARIZE" = 1 ]]; then
  notarize "$DMG"
  retry xcrun stapler staple "$DMG"
  retry xcrun stapler validate "$DMG"
  spctl --assess --type open --context context:primary-signature -vvv "$DMG"
fi

"$WORK/bin/monet" --version
echo "==> done (check the artifacts as shipped with packaging/macos/verify.sh --arch $ARCH)"
ls -lh "$DIST"/monet-"$VERSION"-macos-"$ARCH".*
