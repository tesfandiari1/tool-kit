#!/usr/bin/env bash
#
# Point the desktop app at a conversion service somebody else runs, or put it
# back on the one inside the bundle.
#
# The app converts through its own sidecar and offers no control that changes
# that, because the service ships in `Contents/MacOS` and running one somewhere
# else is a deployment decision rather than a preference. This script is that
# decision: it writes the single file `backend_host::deployment` looks for, and
# removing the file is the whole of the way back.
#
#   backend-override.sh [url]   point the app at url (default: the compose port)
#   backend-override.sh --clear delete the override, back to the sidecar
#   backend-override.sh --show  report which one the app will use
#
# Restart Tool-Kit afterwards. The origin is read per request, so a running app
# follows a new file immediately, but the sidecar it already spawned keeps
# running until the app is relaunched.

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
CRATE_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"

# Read from the config rather than written down twice, so a change to the
# bundle identifier cannot leave this script writing to a directory the app
# does not read.
IDENTIFIER="$(sed -n 's/^[[:space:]]*"identifier"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  "${CRATE_DIR}/tauri.conf.json" | head -n 1)"
[ -n "$IDENTIFIER" ] || { echo "could not read the bundle identifier from tauri.conf.json" >&2; exit 1; }

CONFIG_DIR="${HOME}/Library/Application Support/${IDENTIFIER}"
OVERRIDE="${CONFIG_DIR}/backend-override.json"
# Matches deploy/docker/compose.yaml, which publishes on loopback only.
DEFAULT_URL="http://127.0.0.1:${TOOLKIT_CONVERTER_PORT:-8080}"

show() {
  if [ -f "$OVERRIDE" ]; then
    printf 'manual: %s\n' "$(sed -n 's/.*"url"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$OVERRIDE")"
    printf '  from %s\n' "$OVERRIDE"
  else
    printf 'sidecar: the service inside Tool-Kit.app\n'
    printf '  no %s\n' "$OVERRIDE"
  fi
}

case "${1:-}" in
  --show|-s) show; exit 0 ;;
  --clear|-c)
    rm -f "$OVERRIDE"
    echo "removed the override. Tool-Kit will use its own sidecar again."
    echo "Restart Tool-Kit to stop pointing at the old service."
    exit 0 ;;
  -h|--help) sed -n '2,19p' "${BASH_SOURCE[0]}"; exit 0 ;;
esac

URL="${1:-$DEFAULT_URL}"
# ?* requires a host after the scheme. A bare "http://" would otherwise pass
# this check and write an override parse_override rejects, so the script would
# report success on a url the app refuses.
case "$URL" in
  http://?*|https://?*) : ;;
  *) echo "the backend url must start with http:// or https:// and name a host, got '${URL}'" >&2; exit 2 ;;
esac
# Trailing slash trimmed on this side too. The ledger compares origins by exact
# string, so the app trims as well, and matching here keeps --show honest about
# what the app actually resolved.
URL="${URL%/}"

mkdir -p "$CONFIG_DIR"
# Written whole and renamed, the way settings.json is. The app reads this file
# on every backend request, so a half-written one would be read.
TMP="$(mktemp "${CONFIG_DIR}/backend-override.XXXXXX")"
printf '{\n  "url": "%s"\n}\n' "$URL" > "$TMP"
mv "$TMP" "$OVERRIDE"

printf 'Tool-Kit will use %s\n' "$URL"
printf '  wrote %s\n' "$OVERRIDE"
echo "Restart Tool-Kit so it stops spawning a sidecar it will not use."
echo "Add the service's bearer token in Settings; the app only mints its own."
