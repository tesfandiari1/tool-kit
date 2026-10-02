#!/usr/bin/env bash
#
# Lists each failed run from history.db beside the converter's own verdict on
# that file: engine, classification, fallback reason and OCR page count.
#
# history.db stores no conversion id, so a row pairs with a conversion by the
# SHA-256 of its source file. The converter deletes its own copy once a job
# ends. A source that moved or changed since still prints, with the converter
# columns empty.
#
# Usage: apps/desktop/src-tauri/scripts/trace-failures.sh [N]   (last N failures)
set -euo pipefail

limit="${1:--1}"
[[ "$limit" =~ ^-?[0-9]+$ ]] || { echo "usage: $0 [N]" >&2; exit 64; }

D="$HOME/Library/Application Support/dev.esfandiari.toolkit"
[[ -f "$D/history.db" ]] || { echo "no history.db in $D" >&2; exit 1; }

# (id, sha) per failed row. A missing or unreadable source hashes to NULL.
# ASCII mode separates on 0x1F/0x1E, so any path survives the round trip.
hashes="(NULL, NULL)"
while IFS=$'\x1f' read -r -d $'\x1e' id path; do
  sha=NULL
  if [[ -f "$path" ]] && sum=$(shasum -a 256 < "$path" 2>/dev/null); then
    sha="'${sum%% *}'"
  fi
  hashes+=", ($id, $sha)"
done < <(sqlite3 -readonly -ascii "$D/history.db" "
SELECT id, source_path FROM history WHERE status = 'failed'
ORDER BY finished_at DESC LIMIT $limit")

converter="$D/converter/converter.sqlite"
# The converter runs in WAL mode, and a read-only open with no -shm file fails.
# With the app closed there is no WAL to miss, so read the file as immutable.
mode=immutable=1
[[ -e "$converter-shm" ]] && mode=mode=ro
q="'"
sqlite3 -readonly -header -column "$D/history.db" "
ATTACH 'file:${converter//$q/$q$q}?$mode' AS c;
WITH h(id, sha) AS (VALUES $hashes)
SELECT f.file_name,
       datetime(f.finished_at, 'unixepoch') AS failed_at,
       f.error,
       a.engine_name AS engine,
       a.classification,
       a.fallback_reason,
       json_extract(a.inspection_json, '\$.pagesNeedingOcr') AS ocr_pages,
       json_extract(a.inspection_json, '\$.pageCount') AS pages,
       cv.id AS conversion_id
FROM history f
JOIN h USING (id)
LEFT JOIN c.conversions cv ON cv.source_sha256 = h.sha
LEFT JOIN c.attempts a ON a.id = cv.active_attempt_id
ORDER BY f.finished_at;"
