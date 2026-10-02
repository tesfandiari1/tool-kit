#!/usr/bin/env bash
#
# Lists each failed run from history.db beside the converter's own verdict on
# that file: engine, classification, fallback reason and OCR page count.
#
# history.db stores no conversion id, so a row pairs with a conversion by its
# source bytes. A source that moved or changed since still prints, with the
# converter columns empty.
#
# Usage: apps/desktop/src-tauri/scripts/trace-failures.sh [N]   (last N failures)
set -euo pipefail

limit="${1:--1}"
[[ "$limit" =~ ^-?[0-9]+$ ]] || { echo "usage: $0 [N]" >&2; exit 64; }

D="$HOME/Library/Application Support/dev.esfandiari.toolkit"
[[ -f "$D/history.db" ]] || { echo "no history.db in $D" >&2; exit 1; }

sqlite3 -readonly -header -column "$D/history.db" "
ATTACH 'file:$D/converter/converter.sqlite?mode=ro' AS c;
WITH f AS MATERIALIZED (
  SELECT id, file_name, finished_at, error, readfile(source_path) AS b
  FROM history WHERE status = 'failed'
  ORDER BY finished_at DESC LIMIT $limit
)
SELECT f.file_name,
       datetime(f.finished_at, 'unixepoch') AS failed_at,
       f.error,
       a.engine_name AS engine,
       a.classification,
       a.fallback_reason,
       json_extract(a.inspection_json, '\$.pagesNeedingOcr') AS ocr_pages,
       json_extract(a.inspection_json, '\$.pageCount') AS pages,
       cv.id AS conversion_id
FROM f
LEFT JOIN c.conversions cv
  ON cv.source_byte_length = length(f.b)
 AND readfile('$D/converter/' || cv.source_relative_path) = f.b
LEFT JOIN c.attempts a ON a.id = cv.active_attempt_id
ORDER BY f.finished_at;"
