#!/bin/sh
# Cargo `runner` for local macOS builds: sign the freshly built binary with the
# app's real identity, then exec it.
#
# This only matters for the *legacy* keychain, which is the one the dev loop is
# stuck with: `keychain-access-groups` is a restricted entitlement and needs an
# embedded provisioning profile, which a bare Mach-O has nowhere to put. So dev
# reads fall back to the legacy store, and that store authorises a *code
# identity*, not a path. `cargo run` leaves an ad-hoc, linker-signed binary
# whose designated requirement is its own cdhash, so every rebuild arrives as a
# new app and every key read prompts again. No amount of "Always Allow" survives
# that. The shipped .app carries the profile and never reaches this code path.
#
# Signing with the same Developer ID and the same identifier as the shipped app
# gives the dev binary a designated requirement byte-identical to the installed
# one:
#
#   identifier "dev.esfandiari.toolkit" and anchor apple generic and … subject.OU = <team>
#
# That requirement is stable across rebuilds, so an "Always Allow" sticks, and
# the items the installed app already created are already trusted for it.
#
# Deliberately *not* `--options runtime`: the hardened runtime the release
# bundle carries would need `com.apple.security.get-task-allow` to stay
# debuggable, and the designated requirement does not depend on it either way.
#
# Every failure path still runs the binary. A dev loop that dies because a
# certificate expired would be a worse bug than the prompts this removes.
set -eu

if [ $# -eq 0 ]; then
    echo "dev-run: expected a binary path; cargo passes one as \$1" >&2
    exit 64
fi

BIN=$1
shift

warn() {
    echo "dev-run: $*" >&2
}

# Test and bench binaries live under target/*/deps/. They never read the
# keychain, and cargo already leaves them with a valid ad-hoc signature, so
# they skip the second that signing a 55MB debug binary costs.
case "$BIN" in
*/deps/*) exec "$BIN" "$@" ;;
esac

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
crate_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
repo_root=$(CDPATH= cd -- "$crate_dir/.." && pwd)
conf=$crate_dir/tauri.conf.json

# Read from tauri.conf.json rather than hardcoding. Half the designated
# requirement is this string, so a bundle-id change that this script did not
# follow would bring the prompts back with nothing on screen to explain why.
bundle_identifier() {
    sed -n 's/^[[:space:]]*"identifier"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        "$conf" 2>/dev/null | head -n 1
}

signing_identity() {
    # 1. The environment, the same variable `pnpm tauri build` takes.
    if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
        printf '%s' "$APPLE_SIGNING_IDENTITY"
        return
    fi
    # 2. .env.local, where this repo's signing config already lives (gitignored).
    if [ -f "$repo_root/.env.local" ]; then
        found=$(sed -n 's/^[[:space:]]*\(export[[:space:]]*\)\{0,1\}APPLE_SIGNING_IDENTITY=//p' \
            "$repo_root/.env.local" | tail -n 1 | tr -d '\r' |
            sed 's/^"\(.*\)"$/\1/; s/^'\''\(.*\)'\''$/\1/')
        if [ -n "$found" ]; then
            printf '%s' "$found"
            return
        fi
    fi
    # 3. The first Developer ID in the keychain, by SHA-1 rather than by name so
    #    a second certificate with the same subject cannot be picked by accident.
    security find-identity -v -p codesigning 2>/dev/null |
        awk '/Developer ID Application/ { print $2; exit }'
}

identity=$(signing_identity)
identifier=$(bundle_identifier)
prompts="The keychain will prompt on every rebuild."

if ! command -v codesign >/dev/null 2>&1; then
    warn "codesign not found, install the Xcode command line tools. $prompts"
elif [ -z "$identity" ]; then
    warn "no Developer ID Application certificate found. $prompts"
elif [ -z "$identifier" ]; then
    warn "no \"identifier\" in $conf. $prompts"
else
    # --timestamp=none keeps this offline and fast. A secure timestamp only
    # matters for notarisation, and nothing built here is distributed.
    if ! error=$(codesign --force --sign "$identity" --identifier "$identifier" \
        --timestamp=none "$BIN" 2>&1); then
        warn "codesign failed. $prompts Error: $error"
    fi

    # `--force` drops the old signature before writing the new one. Interrupt it
    # in between and the binary is left with no valid signature at all, which on
    # Apple Silicon the kernel answers with SIGKILL and no message. Verify what
    # actually landed, and put a valid ad-hoc signature back if it is broken:
    # that is what cargo produces anyway, so the dev loop keeps working.
    if ! codesign --verify "$BIN" >/dev/null 2>&1; then
        warn "signature on $BIN is invalid, restoring an ad-hoc one. $prompts"
        codesign --force --sign - "$BIN" >/dev/null 2>&1 ||
            warn "ad-hoc re-sign failed too. $BIN will be killed on launch; rebuild it"
    fi
fi

exec "$BIN" "$@"
