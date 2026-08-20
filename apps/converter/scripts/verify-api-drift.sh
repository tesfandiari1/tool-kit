#!/usr/bin/env bash
#
# The generated client still matches the contract it was generated from.
#
# `src/app/api/schema.ts` is produced from `contract/http/openapi.yaml` by
# `pnpm generate:api`, and ESLint blocks importing it from anywhere but
# `src/app/api/`. What can still go wrong is the file going stale: the contract
# gains a field and nobody regenerates, or someone edits the generated file by
# hand despite the "Do not make direct changes" header. Either way the types
# the app compiles against stop describing the service it calls.
#
# This regenerates into a temporary file and diffs. It never writes into the
# tree and it never asks git anything.
#
# The previous form was `pnpm generate:api && git diff --exit-code
# src/app/api/schema.ts`, which regenerated in place and then asked git whether
# that changed the worktree relative to the index. On a clean checkout the two
# questions have the same answer, so CI was fine. On any working tree holding
# an unstaged edit to schema.ts, a correctly regenerated file still failed the
# gate, and the only way to make it pass was to stage the file. A check that
# reports drift for a file with no drift trains you to ignore it.
#
# Usage: apps/converter/scripts/verify-api-drift.sh

set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/../../.." && pwd -P)"

CONTRACT="${ROOT_DIR}/contract/http/openapi.yaml"
GENERATED="${ROOT_DIR}/apps/desktop/src/app/api/schema.ts"

[ -f "${CONTRACT}" ] || { printf 'FAIL: no contract at %s\n' "${CONTRACT}" >&2; exit 1; }
[ -f "${GENERATED}" ] || { printf 'FAIL: no generated client at %s\n' "${GENERATED}" >&2; exit 1; }

EXPECTED="$(mktemp -t schema-drift)"
# Runs on every exit path, so a failing diff does not leave the file behind.
trap 'rm -f "${EXPECTED}"' EXIT

# openapi-typescript writes no path into its output, so generating somewhere
# else is byte-for-byte what generating in place would have produced.
pnpm --dir "${ROOT_DIR}" exec openapi-typescript "${CONTRACT}" -o "${EXPECTED}" >/dev/null

if diff -u "${GENERATED}" "${EXPECTED}"; then
  printf 'RESULT: PASS, apps/desktop/src/app/api/schema.ts matches the contract\n'
  exit 0
fi

printf '\nFAIL: apps/desktop/src/app/api/schema.ts does not match %s\n' "${CONTRACT}" >&2
printf 'Run `pnpm generate:api` and commit the result.\n' >&2
exit 1
