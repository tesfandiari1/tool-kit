#!/usr/bin/env bash
#
# Post-build checks on the macOS bundle, run after `pnpm tauri build`.
#
# The one that matters most is the designated requirement. An unsigned build
# still produces a working .app, so nothing before this point fails, but its
# requirement is a bare cdhash of that exact binary. The macOS keychain
# authorises against that requirement, so every rebuild arrives as a new app and
# every "Always Allow" the user gave is void: three API keys re-prompted on
# every launch, with nothing on screen to explain it. A Developer ID signature
# makes the requirement identity-based and so stable across versions.
#
# Usage: src-tauri/scripts/verify-release.sh [--notarized]
#
# Without --notarized the stapler and Gatekeeper checks are skipped, which is
# right for a local signed install: locally-built apps are not quarantined.

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
CRATE_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"
BUNDLE="${CRATE_DIR}/target/release/bundle"
APP="${BUNDLE}/macos/Tool-Kit.app"
NOTARIZED="no"

while [ $# -gt 0 ]; do
  case "$1" in
    --notarized) NOTARIZED="yes"; shift ;;
    -h|--help) sed -n '2,17p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

pass() { printf '   PASS %s\n' "$*"; }
fail() { printf '\nFAIL: %s\n' "$*" >&2; exit 1; }

[ -d "$APP" ] || fail "no bundle at ${APP}. Run pnpm tauri build first"

IDENTIFIER="$(sed -n 's/^[[:space:]]*"identifier"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  "${CRATE_DIR}/tauri.conf.json" | head -n 1)"
[ -n "$IDENTIFIER" ] || fail "could not read the bundle identifier from tauri.conf.json"

printf '\n== Signature\n'
# This check runs before `codesign --verify` on purpose. An ad-hoc bundle fails
# both, and "codesign --verify rejected the bundle" says nothing about the
# keychain, which is the consequence the person reading this needs to see.
REQUIREMENT="$(codesign -d -r- "$APP" 2>&1 | sed -n 's/^#* *designated => //p' | head -n 1)"
[ -n "$REQUIREMENT" ] || fail "could not read a designated requirement from ${APP}"
case "$REQUIREMENT" in
  *"identifier \"${IDENTIFIER}\""*"anchor apple generic"*)
    pass "designated requirement is identity-based" ;;
  *cdhash*)
    fail "the bundle is ad-hoc signed, so its designated requirement is a bare
     cdhash of this one binary:

       ${REQUIREMENT}

     The macOS keychain authorises against that requirement, so the next build
     arrives as a different app and every \"Always Allow\" is void: one prompt
     per API key per launch, forever. Build again with APPLE_SIGNING_IDENTITY
     set and the requirement becomes identity-based and stable across versions." ;;
  *)
    fail "unrecognized designated requirement: ${REQUIREMENT}" ;;
esac

codesign --verify --deep --strict --verbose=2 "$APP" 2>/dev/null \
  || fail "codesign --verify rejected the bundle"
pass "codesign --verify"

printf '\n== Bundle metadata\n'
PLIST="${APP}/Contents/Info.plist"
for key in CFBundleShortVersionString LSMinimumSystemVersion NSHumanReadableCopyright LSApplicationCategoryType; do
  value="$(plutil -extract "$key" raw -o - "$PLIST" 2>/dev/null || true)"
  [ -n "$value" ] || fail "Info.plist is missing ${key}"
  pass "${key} = ${value}"
done

printf '\n== Bundled notices\n'
for f in OFL.txt THIRD_PARTY_NOTICES.md; do
  [ -f "${APP}/Contents/Resources/${f}" ] \
    || fail "Resources/${f} is missing, and the OFL requires it to ship beside the fonts"
  pass "Resources/${f}"
done

printf '\n== Dead weight\n'
if find "$APP" -iname '*allery*' -print -quit | grep -q .; then
  fail "the design-system gallery chunk is inside the bundle"
fi
pass "no gallery chunk"

if [ "$NOTARIZED" = "yes" ]; then
  printf '\n== Notarization\n'
  DMG="$(ls -t "${BUNDLE}"/dmg/*.dmg 2>/dev/null | head -n 1 || true)"
  [ -n "${DMG:-}" ] || fail "no DMG found under ${BUNDLE}/dmg"
  xcrun stapler validate "$APP" || fail "the .app has no stapled ticket"
  pass "stapler validate (app)"
  xcrun stapler validate "$DMG" || fail "the DMG has no stapled ticket"
  pass "stapler validate (dmg)"
  # Must be `source=Notarized Developer ID`, not merely `Developer ID`.
  spctl -a -t open --context context:primary-signature -vv "$DMG" 2>&1 \
    | grep -q "source=Notarized Developer ID" \
    || fail "Gatekeeper does not report the DMG as notarized"
  pass "spctl accepts the DMG as notarized"
fi

printf '\nRESULT: PASS\n'
