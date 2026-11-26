#!/usr/bin/env bash
# Check the macOS release artifact exactly as users download it:
#
#   $DIST/monet-<version>-macos-<arch>.dmg   signed, notarized, ticket stapled
#
# Usage: packaging/macos/verify.sh [--arch universal|aarch64|x86_64]
#
# With a real signing identity and notarization credentials in the environment (the same
# MACOS_SIGN_IDENTITY / APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID that package.sh uses), every
# check is strict and a failure exits non-zero. Without them (local ad-hoc builds, or a release run
# whose secrets are missing) only the signature's integrity is checked and the rest is a warning,
# matching package.sh. Nothing here reads or prints a secret's value; it only tests whether one is set.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

ARCH=universal
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal | aarch64 | x86_64) ;;
  *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
esac

DMG="$DIST/monet-$VERSION-macos-$ARCH.dmg"
IDENTITY="${MACOS_SIGN_IDENTITY:--}"
STRICT=0
if [[ "$IDENTITY" != "-" ]] && [[ -n "${APPLE_ID:-}" ]] && [[ -n "${APPLE_PASSWORD:-}" ]] && [[ -n "${APPLE_TEAM_ID:-}" ]]; then
  STRICT=1
fi

FAILED=0
fail() {
  if [[ "$STRICT" = 1 ]]; then
    echo "error: $*" >&2
    FAILED=1
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
trap 'rm -rf "$WORK"' EXIT

# ---- DMG ---------------------------------------------------------------------------------------
echo "==> $(basename "$DMG")"
codesign --verify --strict --verbose=2 "$DMG"
if [[ "$STRICT" = 1 ]]; then
  retry xcrun stapler validate "$DMG" || fail "no notarization ticket stapled to the DMG"
  assess --assess --type open --context context:primary-signature -vv "$DMG" \
    || fail "Gatekeeper rejects the DMG"
fi

if [[ "$FAILED" != 0 ]]; then
  echo "error: macOS artifacts failed verification" >&2
  exit 1
fi

if [[ "$STRICT" = 1 ]]; then
  result="signed (Developer ID, hardened runtime), notarized; DMG stapled"
else
  result="signature intact; Developer ID and notarization not required (no signing secrets)"
fi
echo "==> verified: $result"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  echo "- macOS \`$(basename "$DMG")\`: $result" >>"$GITHUB_STEP_SUMMARY"
fi
