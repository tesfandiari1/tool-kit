#!/usr/bin/env bash
# Fails when a literal colour, shape or face reaches a component, or when a
# token the swap deleted is still referenced. grep only, no dependency.
#
# Exempt: ui/tokens.css owns every literal, and ui/fonts.css owns every
# `font-family:` in an @font-face.
set -euo pipefail

cd "$(dirname "$0")/.."
SRC=src

fail=0

report() {
  local what=$1
  shift
  local hits
  hits=$("$@" || true)
  if [ -n "$hits" ]; then
    echo "lint:tokens: $what"
    echo "$hits"
    echo
    fail=1
  fi
}

scan() {
  grep -rnE "$1" "$SRC" \
    --include='*.css' --include='*.ts' --include='*.tsx' \
    --exclude-dir=node_modules \
    | grep -v '^src/ui/tokens\.css:' \
    | grep -v '^src/ui/fonts\.css:'
}

report "literal colours: use the role layer" \
  scan '#[0-9a-fA-F]{3,8}\b|rgba?\('

report "no shadows, no radii, no blur" \
  scan 'box-shadow|border-radius|backdrop-filter'

report "font families come from a --font-* token" \
  scan 'font-family:[^;]*[",]|fontFamily: "[^v]'

report "deleted tokens" \
  scan 'var\(--(ink|ink-2|ink-3|ink-ghost|accent|accent-hover|accent-active|accent-ink|accent-quiet|live|pass|fault|focus|select|surface|surface-raised|surface-well|surface-solid|surface-scrim|rule-lit|rule-strong|track-mono|weight-medium|weight-normal|font-mono|ease-out|r-control|r-panel|r-pill)\)|var\(--(warm|cobalt)-'

exit $fail
