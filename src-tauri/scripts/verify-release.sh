#!/usr/bin/env bash
#
# Post-build checks on the macOS bundle, run after `pnpm tauri build`.
#
# Two checks earn this script.
#
# The designated requirement. An unsigned build still produces a working .app,
# so nothing before this point fails, but its requirement is a bare cdhash of
# that one binary: Gatekeeper on another Mac rejects it and notarization will
# not touch it. A Developer ID signature makes the requirement identity-based
# and so stable across versions.
#
# The keychain entitlement. Reaching the data protection keychain, the only
# macOS keychain with no access dialogs, takes keychain-access-groups plus an
# embedded provisioning profile that authorises the claim. Miss either and the
# app still builds, still runs and still stores keys, but falls back to the
# legacy keychain, where access is an ACL bound to the code signature and every
# rebuild re-prompts for every key, with nothing on screen to explain it.
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
    -h|--help) sed -n '2,24p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

pass() { printf '   PASS %s\n' "$*"; }
fail() { printf '\nFAIL: %s\n' "$*" >&2; exit 1; }

[ -d "$APP" ] || fail "no bundle at ${APP}. Run pnpm tauri build first"

IDENTIFIER="$(sed -n 's/^[[:space:]]*"identifier"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  "${CRATE_DIR}/tauri.conf.json" | head -n 1)"
[ -n "$IDENTIFIER" ] || fail "could not read the bundle identifier from tauri.conf.json"

# entitlements.plist is the contract. This script only asserts the build
# honoured it, so the team id and the access group live in exactly one place.
# plutil keypaths split on ".", so the reverse-DNS keys need escaping.
ENTITLEMENTS_SRC="${CRATE_DIR}/entitlements.plist"
[ -f "$ENTITLEMENTS_SRC" ] || fail "missing ${ENTITLEMENTS_SRC}"
want() { plutil -extract "$1" raw -o - "$ENTITLEMENTS_SRC" 2>/dev/null; }
WANT_APP_ID="$(want 'com\.apple\.application-identifier')"
WANT_TEAM="$(want 'com\.apple\.developer\.team-identifier')"
WANT_GROUP="$(want 'keychain-access-groups.0')"
[ -n "$WANT_APP_ID" ] && [ -n "$WANT_TEAM" ] && [ -n "$WANT_GROUP" ] \
  || fail "entitlements.plist is missing one of the three keychain keys"

printf '\n== Signature\n'
# This check runs before `codesign --verify` on purpose. An ad-hoc bundle fails
# both, and "codesign --verify rejected the bundle" says nothing about the
# keychain, which is the consequence the person reading this needs to see.
REQUIREMENT="$(codesign -d -r- "$APP" 2>&1 | sed -n 's/^#* *designated => //p' | head -n 1)"
[ -n "$REQUIREMENT" ] || fail "could not read a designated requirement from ${APP}"
case "$REQUIREMENT" in
  *"identifier \"${IDENTIFIER}\""*"anchor apple generic"*"subject.OU] = \"${WANT_TEAM}\""*)
    pass "designated requirement is identity-based and team-scoped" ;;
  *cdhash*)
    fail "the bundle is ad-hoc signed, so its designated requirement is a bare
     cdhash of this one binary:

       ${REQUIREMENT}

     Gatekeeper on any other Mac rejects that, and notarization will not touch
     it. Build again with APPLE_SIGNING_IDENTITY set and the requirement
     becomes identity-based and stable across versions." ;;
  *"identifier \"${IDENTIFIER}\""*"anchor apple generic"*)
    fail "the requirement is identity-based but not scoped to team ${WANT_TEAM}:

       ${REQUIREMENT}

     The data protection keychain matches on team id, so a bundle signed by a
     different team cannot read the keys this one wrote." ;;
  *)
    fail "the requirement does not name ${IDENTIFIER}, so this bundle is stale
     or was signed for a different app:

       ${REQUIREMENT}" ;;
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

printf '\n== Keychain entitlement\n'
# Without all of this the app falls back to the legacy keychain at runtime and
# the only symptom is the dialogs coming back, months later, on a rebuild.
ENT="$(mktemp -t toolkit-entitlements)"
trap 'rm -f "${ENT}"' EXIT
codesign -d --entitlements - --xml "$APP" 2>/dev/null > "$ENT" || true
[ -s "$ENT" ] || fail "the bundle carries no entitlements. Check that
     bundle.macOS.entitlements points at entitlements.plist in tauri.conf.json"

got() { plutil -extract "$1" raw -o - "$ENT" 2>/dev/null || true; }
check() {
  [ "$2" = "$3" ] || fail "entitlement ${1} is '${2:-<missing>}', expected '${3}'"
  pass "${1} = ${2}"
}
check "com.apple.application-identifier" \
  "$(got 'com\.apple\.application-identifier')" "$WANT_APP_ID"
check "com.apple.developer.team-identifier" \
  "$(got 'com\.apple\.developer\.team-identifier')" "$WANT_TEAM"
check "keychain-access-groups" "$(got 'keychain-access-groups.0')" "$WANT_GROUP"

PROFILE="${APP}/Contents/embedded.provisionprofile"
[ -f "$PROFILE" ] || fail "no Contents/embedded.provisionprofile. keychain-access-groups
     is a restricted entitlement and is only honoured when a profile authorises
     it. Add bundle.macOS.files to tauri.conf.json and drop the profile at
     ${CRATE_DIR}/embedded.provisionprofile"

# The allowlist entry is usually the wildcard TEAM.*, so this is a match, not a
# string compare. Expiry is checked here because macOS evaluates the profile at
# every launch: an expired one stops the app dead, long after the build passed.
security cms -D -i "$PROFILE" 2>/dev/null | python3 -c '
import datetime, plistlib, sys
want_group, want_app_id = sys.argv[1], sys.argv[2]
try:
    profile = plistlib.loads(sys.stdin.buffer.read())
except Exception as error:
    sys.exit(f"could not decode the provisioning profile: {error}")
allowed = profile.get("Entitlements", {})
groups = allowed.get("keychain-access-groups") or []
if not any(g == want_group or (g.endswith(".*") and want_group.startswith(g[:-1]))
           for g in groups):
    sys.exit(f"the profile allows keychain groups {groups}, which do not cover {want_group}")
app_id = allowed.get("com.apple.application-identifier")
if app_id != want_app_id:
    sys.exit(f"the profile is for app id {app_id!r}, not {want_app_id!r}")
expires = profile.get("ExpirationDate")
if expires is None:
    sys.exit("the profile carries no ExpirationDate, so macOS cannot evaluate it")
# plistlib returns a naive UTC datetime, so both sides are made aware here.
if expires.replace(tzinfo=datetime.timezone.utc) < datetime.datetime.now(datetime.timezone.utc):
    sys.exit(f"the provisioning profile expired on {expires:%Y-%m-%d}")
print(f"   PASS profile authorises {want_group} until {expires:%Y-%m-%d}")
' "$WANT_GROUP" "$WANT_APP_ID" || fail "embedded.provisionprofile does not authorise the entitlement"

printf '\nRESULT: PASS\n'
