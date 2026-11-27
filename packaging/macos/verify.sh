#!/usr/bin/env bash
# Check the macOS release artifact exactly as users download it:
#
#   $DIST/monet-<version>-macos-<arch>.dmg   signed, notarized, ticket stapled
#
# Usage: packaging/macos/verify.sh [--arch universal|aarch64|x86_64] [--require-signing]
#
# With signing secrets in the environment, every signature/notary check is strict and a failure
# exits non-zero. Without them (local ad-hoc builds) the bundle structure, architecture, plist and
# resources are still checked; signing and notarization are warnings. --require-signing makes
# missing secrets an error, which is what release workflows use.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

ARCH=universal
REQUIRE_SIGNING="${MONET_REQUIRE_SIGNING:-0}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    --require-signing) REQUIRE_SIGNING=1; shift ;;
    -h | --help) sed -n '2,18p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) WANT_ARCHS="arm64 x86_64" ;;
  aarch64) WANT_ARCHS="arm64" ;;
  x86_64) WANT_ARCHS="x86_64" ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

DMG="$DIST/monet-$VERSION-macos-$ARCH.dmg"
SHORT_VERSION="${VERSION%%-*}"
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
STRICT=0
if [[ "$IDENTITY" != "-" ]] && [[ -n "${APPLE_ID:-}" ]] && [[ -n "${APPLE_PASSWORD:-}" ]] && [[ -n "${APPLE_TEAM_ID:-}" ]]; then
  STRICT=1
fi
if [[ "$REQUIRE_SIGNING" = 1 ]]; then
  if [[ "$IDENTITY" = "-" || -z "${APPLE_ID:-}" || -z "${APPLE_PASSWORD:-}" || -z "${APPLE_TEAM_ID:-}" ]]; then
    echo "error: --require-signing needs MACOS_SIGN_IDENTITY, APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID" >&2
    exit 2
  fi
  STRICT=1
fi

FAILED=0
fail() {
  echo "error: $*" >&2
  FAILED=1
}
# Signature/notarization gaps are warnings for an intentional local ad-hoc build and errors when
# signing secrets or --require-signing make this a release check.
soft_fail() {
  if [[ "$STRICT" = 1 ]]; then
    fail "$*"
  else
    warn "$* (expected without signing secrets)"
  fi
}

# spctl's online ticket lookup can lag a few seconds behind notarytool's "Accepted".
assess() {
  local out attempt tries=1
  [[ "$STRICT" = 1 ]] && tries=5
  for attempt in $(seq "$tries"); do
    if out="$(spctl "$@" 2>&1)"; then
      printf '%s\n' "$out"
      return 0
    fi
    [[ "$attempt" = "$tries" ]] || sleep 10
  done
  printf '%s\n' "$out"
  return 1
}

if [[ ! -f "$DMG" ]]; then
  echo "error: missing $DMG" >&2
  exit 1
fi

WORK="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/monet-verify.XXXXXX")"
MNT="$WORK/mnt"
MOUNTED=0
cleanup() {
  if [[ "$MOUNTED" = 1 ]]; then hdiutil detach "$MNT" >/dev/null 2>&1 || hdiutil detach -force "$MNT" >/dev/null 2>&1 || true; fi
  rm -rf "$WORK"
}
trap cleanup EXIT

# ---- DMG itself --------------------------------------------------------------------------------
echo "==> $(basename "$DMG")"
codesign --verify --strict --verbose=2 "$DMG"
if [[ "$STRICT" = 1 ]]; then
  retry xcrun stapler validate "$DMG" || fail "no notarization ticket stapled to the DMG"
  assess --assess --type open --context context:primary-signature -vv "$DMG" \
    || fail "Gatekeeper rejects the DMG"
fi

# ---- mounted app ---------------------------------------------------------------------------------
echo "==> mounting $DMG"
mkdir -p "$MNT"
hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG" >/dev/null
MOUNTED=1

APP="$MNT/Monet.app"
EXE="$APP/Contents/MacOS/Monet"
if [[ ! -d "$APP" ]]; then
  echo "error: $(basename "$DMG") has no Monet.app" >&2
  exit 1
fi
if [[ ! -x "$EXE" ]]; then
  echo "error: Monet.app has no executable Contents/MacOS/Monet" >&2
  exit 1
fi
# The DMG has exactly the app and the drag-to-Applications link. No CLI or stray files.
for entry in "$MNT"/* "$MNT"/.[!.]*; do
  [[ -e "$entry" ]] || continue
  case "$(basename "$entry")" in
    Applications | Monet.app) ;;
    *) echo "error: unexpected item in DMG: $(basename "$entry")" >&2; exit 1 ;;
  esac
done
if [[ ! -L "$MNT/Applications" ]]; then
  echo "error: DMG is missing the Applications link" >&2
  exit 1
fi
if [[ "$(ls -A "$APP/Contents/MacOS")" != "Monet" ]]; then
  echo "error: Contents/MacOS must contain only Monet (no CLI)" >&2
  exit 1
fi

archs="$(lipo -archs "$EXE")"
actual="$(printf '%s\n' $archs | sort | tr '\n' ' ' | sed 's/ $//')"
want="$(printf '%s\n' $WANT_ARCHS | sort | tr '\n' ' ' | sed 's/ $//')"
[[ "$actual" = "$want" ]] || fail "Monet has architectures '$archs', expected '$WANT_ARCHS'"

codesign --verify --strict --deep --verbose=2 "$APP"
details="$(codesign -d --verbose=2 "$APP" 2>&1)"
printf '%s\n' "$details" | grep -E '^(Identifier|Format|Authority|Timestamp|TeamIdentifier)[= ]' || true
printf '%s\n' "$details" | grep -q '^Identifier=app.monet.design' || fail "bundle identifier is not app.monet.design"
if [[ "$STRICT" = 1 ]]; then
  printf '%s\n' "$details" | grep -q '^Authority=Developer ID Application:' || soft_fail "app is not signed with a Developer ID Application identity"
  printf '%s\n' "$details" | grep -q '^Timestamp=' || soft_fail "app signature has no secure timestamp"
  printf '%s\n' "$details" | grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' || soft_fail "app is not signed with the hardened runtime"
  if [[ -n "${APPLE_TEAM_ID:-}" ]]; then
    printf '%s\n' "$details" | grep -qx "TeamIdentifier=$APPLE_TEAM_ID" || soft_fail "app is signed by a different team"
  fi
fi

plist="$APP/Contents/Info.plist"
value() { plutil -extract "$1" raw -o - "$plist"; }
[[ "$(value CFBundleIdentifier)" = "app.monet.design" ]] || fail "Info.plist CFBundleIdentifier is not app.monet.design"
[[ "$(value CFBundleExecutable)" = "Monet" ]] || fail "Info.plist CFBundleExecutable is not Monet"
[[ "$(value CFBundleIconFile)" = "Monet" ]] || fail "Info.plist CFBundleIconFile is not Monet"
[[ "$(value CFBundleShortVersionString)" = "$SHORT_VERSION" ]] || fail "Info.plist CFBundleShortVersionString is not $SHORT_VERSION"
[[ "$(value CFBundleVersion)" = "$SHORT_VERSION" ]] || fail "Info.plist CFBundleVersion is not $SHORT_VERSION"
[[ "$(value LSMinimumSystemVersion)" = "11.0" ]] || fail "Info.plist LSMinimumSystemVersion is not 11.0"

# ---- runtime resources ---------------------------------------------------------------------------
RES="$APP/Contents/Resources"
[[ -f "$RES/Monet.icns" ]] || fail "missing Contents/Resources/Monet.icns"
[[ -f "$RES/NOTICE" ]] || fail "missing Contents/Resources/NOTICE"
[[ -f "$RES/ATTRIBUTION.md" ]] || fail "missing Contents/Resources/ATTRIBUTION.md"
[[ -f "$RES/assets/devices/devices.json" ]] || fail "missing Contents/Resources/assets/devices/devices.json"
[[ -f "$RES/assets/brushes/monet-defaults.abr" ]] || fail "missing Contents/Resources/assets/brushes/monet-defaults.abr"
[[ -f "$RES/assets/wgsl/glass.wgsl" ]] || fail "missing Contents/Resources/assets/wgsl/glass.wgsl"
device_files="$(find "$RES/assets/devices" -type f | wc -l | tr -d ' ')"
[[ "$device_files" = "156" ]] || fail "device catalogue has $device_files files, expected 156"

# ---- executable smoke ----------------------------------------------------------------------------
echo "==> Monet --version"
version_out="$("$EXE" --version 2>&1)"
printf '%s\n' "$version_out"
[[ "$version_out" = "Monet $VERSION" ]] || fail "Monet --version printed '$version_out', expected 'Monet $VERSION'"

# ---- app alone, outside the DMG -------------------------------------------------------------
if [[ "$STRICT" = 1 ]]; then
  retry xcrun stapler validate "$APP" || fail "no notarization ticket stapled to Monet.app"
  assess --assess --type execute -vv "$APP" || fail "Gatekeeper rejects Monet.app"
  printf '%s\n' "$(spctl --assess --type execute -vv "$APP" 2>&1)" | grep -q 'source=Notarized Developer ID' \
    || fail "Gatekeeper accepts Monet.app but not as notarized"
fi

if [[ "$FAILED" != 0 ]]; then
  echo "error: macOS artifacts failed verification" >&2
  exit 1
fi

if [[ "$STRICT" = 1 ]]; then
  result="signed (Developer ID, hardened runtime), notarized; DMG and app stapled; resources complete"
else
  result="structure, architecture, plist, resources and version intact; Developer ID and notarization not required (no signing secrets)"
fi
echo "==> verified: $result"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  echo "- macOS \`$(basename "$DMG")\`: $result" >>"$GITHUB_STEP_SUMMARY"
fi
