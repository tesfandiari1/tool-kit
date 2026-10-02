#!/usr/bin/env bash
#
# Post-build checks on the macOS bundle, run after `pnpm tauri build`.
#
# Three checks earn this script.
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
# The sidecar signatures. The bundler copies the conversion service and its
# three workers into Contents/MacOS and signs each one on its own. A sidecar
# that missed --options runtime passes codesign --verify --deep --strict here
# and is rejected by the notary an hour later, taking the whole submission.
#
# Usage: apps/desktop/src-tauri/scripts/verify-release.sh [--notarized]
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
    -h|--help) sed -n '2,28p' "${BASH_SOURCE[0]}"; exit 0 ;;
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
want() { plutil -extract "$1" raw -o - "$ENTITLEMENTS_SRC" 2>/dev/null || true; }
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

printf '\n== Sidecars\n'
# --deep --strict above already passed, and it says nothing about any of this.
# Each sidecar is signed on its own by the bundler, so each carries its own
# flags, its own team and its own timestamp, and the app's signature does not
# cover a single one of those three.
for sidecar in tool-kit-converter tool-kit-pdf-worker tool-kit-vision-worker tool-kit-audio-worker; do
  BIN="${APP}/Contents/MacOS/${sidecar}"
  [ -f "$BIN" ] || fail "Contents/MacOS/${sidecar} is missing. The app spawns the
     conversion service by name beside its own executable, so this bundle has no
     local backend at all: every Convert job fails on any machine without Docker.
     Run pnpm sidecars, then build again."
  [ -x "$BIN" ] || fail "Contents/MacOS/${sidecar} is not executable, so the spawn
     dies with permission denied and the local backend never binds a port"

  INFO="$(codesign -d --verbose=4 "$BIN" 2>&1)" \
    || fail "Contents/MacOS/${sidecar} carries no signature. macOS SIGKILLs an
     unsigned helper on Apple Silicon with no message, which the converter
     reports as a worker handshake failure that never mentions signing."

  FLAGS="$(printf '%s\n' "$INFO" | sed -n 's/^CodeDirectory .*\(flags=[^ ]*\).*/\1/p' | head -n 1)"
  [ -n "$FLAGS" ] || fail "could not read CodeDirectory flags for ${sidecar}"
  case "$FLAGS" in
    *adhoc*)
      fail "${sidecar} is ad-hoc signed (${FLAGS}), so it is not covered by the
     Developer ID identity the app carries. Notarization refuses the submission
     and Gatekeeper rejects the app on every Mac but this one." ;;
  esac
  case "$FLAGS" in
    *runtime*) : ;;
    *)
      fail "${sidecar} was signed without the hardened runtime (${FLAGS}). Every
     local check passes on a bundle like this, including codesign --verify
     --deep --strict, and the notary rejects the whole submission an hour later." ;;
  esac

  TEAM="$(printf '%s\n' "$INFO" | sed -n 's/^TeamIdentifier=//p' | head -n 1)"
  [ "$TEAM" = "$WANT_TEAM" ] || fail "${sidecar} is signed by team
     '${TEAM:-<missing>}', not ${WANT_TEAM}. A helper signed by another team
     breaks the app's seal and cannot reach the keys this team's app wrote."

  STAMP="$(printf '%s\n' "$INFO" | sed -n 's/^Timestamp=//p' | head -n 1)"
  [ -n "$STAMP" ] || fail "${sidecar} has no secure timestamp, only a local signing
     time. The notary requires one, and without it the signature stops verifying
     the day the certificate expires rather than outliving it."

  pass "${sidecar}: ${FLAGS}, team ${TEAM}, timestamped ${STAMP}"
done

printf '\n== Nested code\n'
# Contents/MacOS is sealed as nested code, so it holds executables and nothing
# else. A resource that lands there breaks the seal on the user's machine while
# every check on the build machine still passes.
NESTED=0
while IFS= read -r entry; do
  KIND="$(file -b "$entry")"
  case "$KIND" in
    *Mach-O*) NESTED=$((NESTED + 1)) ;;
    *)
      fail "Contents/MacOS/${entry#"${APP}/Contents/MacOS/"} is ${KIND}, not Mach-O.
     That directory is sealed as nested code and only executables belong in it.
     Data files go to Contents/Resources, which is where tauri.conf.json's
     bundle.resources map puts them." ;;
  esac
done < <(find "${APP}/Contents/MacOS" -mindepth 1 \! -type d)
pass "${NESTED} Mach-O files under Contents/MacOS and nothing else"

printf '\n== Sidecar entitlements\n'
# tauri-cli appends --entitlements to every sign target, so restoring
# bundle.macOS.entitlements hands the app's entitlements to each helper too.
for BIN in "${APP}"/Contents/MacOS/*; do
  [ "$(basename "$BIN")" = "tool-kit" ] && continue
  SIDE_ENT="$(codesign -d --entitlements - --xml "$BIN" 2>/dev/null || true)"
  case "$SIDE_ENT" in
    *'<key>'*)
      fail "Contents/MacOS/$(basename "$BIN") carries entitlements. Only the app
     binary may. tauri-cli signs every target with bundle.macOS.entitlements, so
     re-sign the helpers without them before notarizing." ;;
  esac
done
pass "no sidecar carries entitlements"

printf '\n== Converter resources\n'
# validate_cmaps in apps/converter/src/engines/pdf_inspector.rs checks these
# four sentinels and no others, so a copy that drops the remaining 165 files
# starts clean and silently loses ToUnicode mapping for every other CJK
# ordering. Nothing at runtime reports it.
BCMAPS="${APP}/Contents/Resources/pdf-inspector/bcmaps"
[ -d "$BCMAPS" ] || fail "no Contents/Resources/pdf-inspector/bcmaps. The converter
     falls back to the crate's build-time path, which exists on this machine and
     on no user's, and CJK PDFs come out with unmapped text and no error"
for cmap in Adobe-CNS1-UCS2.bcmap Adobe-GB1-UCS2.bcmap Adobe-Japan1-UCS2.bcmap Adobe-Korea1-UCS2.bcmap; do
  [ -s "${BCMAPS}/${cmap}" ] \
    || fail "bcmaps/${cmap} is missing or empty, so the PDF engine refuses to start
     and the converter comes up with no pdf-inspector engine at all"
done
# The sentinels do not catch a truncated copy, and the count is the only thing
# that does. It is compared against what build-sidecars.sh staged rather than a
# number written down here, so a pdf-inspector bump moves both at once.
STAGED="${CRATE_DIR}/resources/pdf-inspector/bcmaps"
[ -d "$STAGED" ] || fail "no ${STAGED} to compare the bundled CMaps against.
     Run pnpm sidecars, which stages the set this bundle should carry"
WANT_CMAPS="$(find "$STAGED" -type f | wc -l | tr -d ' ')"
GOT_CMAPS="$(find "$BCMAPS" -type f | wc -l | tr -d ' ')"
[ "$GOT_CMAPS" = "$WANT_CMAPS" ] || fail "the bundle carries ${GOT_CMAPS} CMap files and
     pnpm sidecars staged ${WANT_CMAPS}. validate_cmaps reads four of them at startup, so a
     partial copy starts clean and silently loses ToUnicode mapping for every
     ordering whose file did not make it"
pass "bcmaps: ${GOT_CMAPS} files matching the staged set, four CMap sentinels non-empty"

# The audio worker sets ModelHub.offlineMode and loads the diarizer from an
# explicit directory, so a missing or partial model set is not a download at
# runtime: the worker refuses to boot and the converter advertises no audio at
# all. The count is compared against what build-sidecars.sh staged, the same
# way as the CMaps, so a FluidAudio model bump moves both at once.
DIARIZER="${APP}/Contents/Resources/fluidaudio/speaker-diarization-coreml"
[ -s "${DIARIZER}/manifest.json" ] || fail "no Contents/Resources/fluidaudio/speaker-diarization-coreml/manifest.json.
     The audio worker loads the diarizer from the bundle and never goes online,
     so this bundle transcribes nothing: every Transcribe job on the backend
     route fails at worker startup"
STAGED_DIARIZER="${CRATE_DIR}/resources/fluidaudio/speaker-diarization-coreml"
[ -d "$STAGED_DIARIZER" ] || fail "no ${STAGED_DIARIZER} to compare the bundled models
     against. Run pnpm sidecars, which stages the set this bundle should carry"
WANT_MODELS="$(find "$STAGED_DIARIZER" -type f | wc -l | tr -d ' ')"
GOT_MODELS="$(find "$DIARIZER" -type f | wc -l | tr -d ' ')"
[ "$GOT_MODELS" = "$WANT_MODELS" ] || fail "the bundle carries ${GOT_MODELS} diarizer files and
     pnpm sidecars staged ${WANT_MODELS}. A CoreML bundle missing one weight file loads
     with an error the worker reports as a failed job, on every audio file"
# The three ThirdPartyLicenses files cover code linked into the worker, not the
# models: fastcluster is BSD and its notice must ship with the binary.
for notice in FluidAudio-Apache-2.0.txt speaker-diarization-CC-BY-4.0.txt \
  FluidAudio-ThirdPartyLicenses/fastcluster-LICENSE.md \
  FluidAudio-ThirdPartyLicenses/vbx-LICENSE.md \
  FluidAudio-ThirdPartyLicenses/NemoTextProcessing-LICENSE.md; do
  [ -s "${APP}/Contents/Resources/fluidaudio/${notice}" ] \
    || fail "Resources/fluidaudio/${notice} is missing or empty, and its licence
     requires it to ship beside the code it covers"
done
pass "fluidaudio: ${GOT_MODELS} model files matching the staged set, five licence texts non-empty"

printf '\n== Bundle metadata\n'
PLIST="${APP}/Contents/Info.plist"
for key in CFBundleShortVersionString LSMinimumSystemVersion NSHumanReadableCopyright LSApplicationCategoryType; do
  value="$(plutil -extract "$key" raw -o - "$PLIST" 2>/dev/null || true)"
  [ -n "$value" ] || fail "Info.plist is missing ${key}"
  pass "${key} = ${value}"
done

printf '\n== Bundled notices\n'
# Two for the fonts, two for the PDF engine: the pdf-inspector crate is MIT and
# its bcmaps carry Adobe's own terms. Both licences require the notice to ship
# with the code, and this bundle now ships all four.
for f in OFL.txt THIRD_PARTY_NOTICES.md pdf-inspector-MIT.txt adobe-bcmaps.txt; do
  [ -f "${APP}/Contents/Resources/${f}" ] \
    || fail "Resources/${f} is missing, and its licence requires it to ship beside
     the code it covers"
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
