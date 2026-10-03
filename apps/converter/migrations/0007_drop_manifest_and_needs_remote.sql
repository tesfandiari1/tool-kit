-- no-transaction
-- Drop the manifest artifact, the `needs_remote` status and the profile.
--
-- A succeeded attempt publishes `result.md` alone. The attempt records the
-- Markdown's size and digest at finalizing, which is what startup recovery
-- checks a publication against before the success commit. An old publication
-- keeps its manifest.json on disk, and validation tolerates it.
--
-- No remote leg exists, so an engine that gave up is a failed job with its
-- reason as the failure code. Old `needs_remote` rows become exactly that, with
-- the message a new one gets. The profile changed nothing and goes. The
-- request fingerprint still hashes `local_only`, so an old request replays.
--
-- Status, state and kind are CHECKs, so all three tables are rebuilt and
-- copied. Foreign keys are disabled for the rebuild and re-enabled after; the
-- copy preserves rowids, so attempts.queue_seq keeps its AUTOINCREMENT
-- high-water mark. Both indexes die with their tables and are recreated.

PRAGMA foreign_keys = OFF;

-- The rebuild is one transaction. sqlx runs this file in autocommit, so
-- without it a kill between DROP and RENAME leaves an unbootable schema.
BEGIN;

-- The old CHECKs admit `failed`, so the rows convert in place first.
UPDATE attempts
SET state = 'failed',
    failure_code = COALESCE(NULLIF(fallback_reason, ''), 'needs_remote'),
    failure_message = 'This file could not be converted on this Mac because '
        || CASE fallback_reason
               WHEN 'scanned_pdf' THEN 'every page is a scanned image.'
               WHEN 'image_based_pdf' THEN 'every page is a scanned image.'
               WHEN 'mixed_pdf' THEN 'some pages are scanned images.'
               WHEN 'ocr_required' THEN 'the text layer is missing or unreadable.'
               WHEN 'garbled_text' THEN 'the text layer is garbled.'
               WHEN 'local_quality_failed' THEN 'no readable text was found.'
               WHEN 'output_too_large' THEN 'the result was too large.'
               ELSE 'the local engine gave up.'
           END
WHERE state = 'needs_remote';

UPDATE conversions
SET status = 'failed',
    failure_code = COALESCE(
        (SELECT a.failure_code FROM attempts AS a
         WHERE a.id = conversions.active_attempt_id),
        'needs_remote'),
    failure_message = COALESCE(
        (SELECT a.failure_message FROM attempts AS a
         WHERE a.id = conversions.active_attempt_id),
        'This file could not be converted on this Mac.')
WHERE status = 'needs_remote';

CREATE TABLE conversions_new (
    id                    TEXT PRIMARY KEY NOT NULL
                                  CHECK(length(id) = 36 AND id = lower(id)),
    client_run_id         TEXT NOT NULL
                                  CHECK(length(client_run_id) = 36
                                    AND client_run_id = lower(client_run_id)),
    auth_scope            TEXT NOT NULL
                                  CHECK(auth_scope = 'bootstrap'),
    idempotency_key_hash  TEXT NOT NULL
                                  CHECK(length(idempotency_key_hash) = 64
                                    AND idempotency_key_hash
                                        NOT GLOB '*[^0-9a-f]*'),
    request_fingerprint   TEXT NOT NULL
                                  CHECK(length(request_fingerprint) = 64
                                    AND request_fingerprint
                                        NOT GLOB '*[^0-9a-f]*'),
    status                TEXT NOT NULL
                                  CHECK(status IN (
                                      'queued',
                                      'converting_local',
                                      'finalizing',
                                      'succeeded',
                                      'failed'
                                  )),
    source_relative_path  TEXT NOT NULL,
    source_media_type     TEXT NOT NULL
                                  CHECK(source_media_type IN (
                                      'application/pdf',
                                      'application/msword',
                                      'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
                                      'application/vnd.ms-powerpoint',
                                      'application/vnd.openxmlformats-officedocument.presentationml.presentation',
                                      'application/vnd.ms-excel',
                                      'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
                                      'application/vnd.oasis.opendocument.text',
                                      'application/vnd.oasis.opendocument.spreadsheet',
                                      'application/vnd.oasis.opendocument.presentation',
                                      'application/rtf',
                                      'text/csv',
                                      'application/vnd.ms-word.document.macroEnabled.12',
                                      'application/vnd.ms-excel.sheet.macroEnabled.12',
                                      'application/vnd.ms-powerpoint.presentation.macroEnabled.12',
                                      'application/vnd.openxmlformats-officedocument.presentationml.slideshow',
                                      'application/vnd.ms-powerpoint.slideshow.macroEnabled.12',
                                      'application/epub+zip',
                                      'image/png',
                                      'image/jpeg',
                                      'image/webp',
                                      'image/tiff',
                                      'image/gif',
                                      'image/bmp',
                                      'audio/wav',
                                      'audio/mp4',
                                      'video/mp4',
                                      'video/quicktime',
                                      'audio/mpeg',
                                      'audio/flac'
                                  )),
    source_byte_length    INTEGER NOT NULL
                                  CHECK(source_byte_length > 0),
    source_sha256         TEXT NOT NULL
                                  CHECK(length(source_sha256) = 64
                                    AND source_sha256
                                        NOT GLOB '*[^0-9a-f]*'),
    active_attempt_id     TEXT,
    route                 TEXT,
    reason_codes_json     TEXT NOT NULL DEFAULT '[]'
                                  CHECK(length(reason_codes_json) <= 8192
                                    AND json_valid(reason_codes_json)
                                    AND json_type(reason_codes_json) = 'array'),
    warnings_json         TEXT NOT NULL DEFAULT '[]'
                                  CHECK(length(warnings_json) <= 65536
                                    AND json_valid(warnings_json)
                                    AND json_type(warnings_json) = 'array'),
    failure_code          TEXT,
    failure_message       TEXT,
    origin_request_id     TEXT NOT NULL,
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL,
    ocr_language_correction INTEGER NOT NULL DEFAULT 1
                                  CHECK(ocr_language_correction IN (0, 1)),
    ocr_custom_words        TEXT NOT NULL DEFAULT ''
                                  CHECK(length(ocr_custom_words) <= 256),
    speaker_count           INTEGER
                                  CHECK(speaker_count IS NULL
                                    OR speaker_count BETWEEN 1 AND 20),

    UNIQUE(auth_scope, idempotency_key_hash),

    CHECK(length(source_relative_path) <= 512
      AND source_relative_path = 'jobs/' || id || '/source/input'),
    CHECK(length(origin_request_id) BETWEEN 1 AND 128),
    CHECK(route IS NULL OR length(route) BETWEEN 1 AND 128),
    CHECK(failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128),
    CHECK(failure_message IS NULL OR length(failure_message) BETWEEN 1 AND 1024),
    CHECK((failure_code IS NULL) = (failure_message IS NULL)),

    FOREIGN KEY(id, active_attempt_id)
        REFERENCES attempts(conversion_id, id)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;

INSERT INTO conversions_new (
    id, client_run_id, auth_scope, idempotency_key_hash, request_fingerprint,
    status, source_relative_path, source_media_type, source_byte_length,
    source_sha256, active_attempt_id, route, reason_codes_json, warnings_json,
    failure_code, failure_message, origin_request_id, created_at, updated_at,
    ocr_language_correction, ocr_custom_words, speaker_count
)
SELECT
    id, client_run_id, auth_scope, idempotency_key_hash, request_fingerprint,
    status, source_relative_path, source_media_type, source_byte_length,
    source_sha256, active_attempt_id, route, reason_codes_json, warnings_json,
    failure_code, failure_message, origin_request_id, created_at, updated_at,
    ocr_language_correction, ocr_custom_words, speaker_count
FROM conversions;
DROP TABLE conversions;
ALTER TABLE conversions_new RENAME TO conversions;

CREATE TABLE attempts_new (
    queue_seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    id                    TEXT NOT NULL UNIQUE
                                  CHECK(length(id) = 36 AND id = lower(id)),
    conversion_id         TEXT NOT NULL,
    attempt_number        INTEGER NOT NULL
                                  CHECK(attempt_number >= 1),
    state                 TEXT NOT NULL
                                  CHECK(state IN (
                                      'queued',
                                      'converting_local',
                                      'finalizing',
                                      'succeeded',
                                      'failed',
                                      'interrupted'
                                  )),
    recovery_count        INTEGER NOT NULL DEFAULT 0
                                  CHECK(recovery_count >= 0),
    engine_name           TEXT,
    engine_version        TEXT,
    route                 TEXT,
    classification        TEXT
                                  CHECK(classification IS NULL OR
                                        classification IN (
                                            'text_based',
                                            'scanned',
                                            'image_based',
                                            'mixed',
                                            'structured_document',
                                            'audio'
                                        )),
    inspection_json       TEXT
                                  CHECK(inspection_json IS NULL OR (
                                      length(inspection_json) <= 1048576
                                      AND json_valid(inspection_json)
                                      AND json_type(inspection_json) = 'object'
                                  )),
    reason_codes_json     TEXT NOT NULL DEFAULT '[]'
                                  CHECK(length(reason_codes_json) <= 8192
                                    AND json_valid(reason_codes_json)
                                    AND json_type(reason_codes_json) = 'array'),
    warnings_json         TEXT NOT NULL DEFAULT '[]'
                                  CHECK(length(warnings_json) <= 65536
                                    AND json_valid(warnings_json)
                                    AND json_type(warnings_json) = 'array'),
    fallback_reason       TEXT,
    failure_code          TEXT,
    failure_message       TEXT,
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL,
    started_at            TEXT,
    finished_at           TEXT,
    markdown_byte_length  INTEGER
                                  CHECK(markdown_byte_length IS NULL
                                    OR markdown_byte_length > 0),
    markdown_sha256       TEXT
                                  CHECK(markdown_sha256 IS NULL OR (
                                      length(markdown_sha256) = 64
                                      AND markdown_sha256
                                          NOT GLOB '*[^0-9a-f]*'
                                  )),

    UNIQUE(conversion_id, attempt_number),
    UNIQUE(conversion_id, id),

    CHECK((engine_name IS NULL) = (engine_version IS NULL)),
    CHECK(engine_name IS NULL OR length(engine_name) BETWEEN 1 AND 128),
    CHECK(engine_version IS NULL OR length(engine_version) BETWEEN 1 AND 128),
    CHECK(route IS NULL OR length(route) BETWEEN 1 AND 128),
    CHECK(classification IS NULL OR length(classification) <= 64),
    CHECK(fallback_reason IS NULL OR length(fallback_reason) <= 128),
    CHECK(failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128),
    CHECK(failure_message IS NULL OR length(failure_message) BETWEEN 1 AND 1024),
    CHECK((failure_code IS NULL) = (failure_message IS NULL)),

    FOREIGN KEY(conversion_id)
        REFERENCES conversions(id)
        ON DELETE CASCADE
) STRICT;

-- Positional, as in 0004. The two trailing NULLs are the Markdown digest no
-- attempt before this migration recorded.
INSERT INTO attempts_new SELECT *, NULL, NULL FROM attempts;
DROP TABLE attempts;
ALTER TABLE attempts_new RENAME TO attempts;

CREATE TABLE artifacts_new (
    attempt_id            TEXT NOT NULL,
    kind                  TEXT NOT NULL
                                  CHECK(kind = 'markdown'),
    relative_path         TEXT NOT NULL,
    media_type            TEXT NOT NULL,
    byte_length           INTEGER NOT NULL
                                  CHECK(byte_length > 0),
    sha256                TEXT NOT NULL
                                  CHECK(length(sha256) = 64
                                    AND sha256 NOT GLOB '*[^0-9a-f]*'),
    created_at            TEXT NOT NULL,

    PRIMARY KEY(attempt_id, kind),

    FOREIGN KEY(attempt_id)
        REFERENCES attempts(id)
        ON DELETE CASCADE,

    CHECK(length(relative_path) <= 512
      AND substr(relative_path, 1, 5) = 'jobs/'
      AND instr(relative_path, '..') = 0
      AND instr(relative_path, '//') = 0
      AND instr(
          relative_path,
          '/attempts/' || attempt_id || '/artifacts/'
      ) > 5
      AND relative_path =
          substr(relative_path, 1, length(relative_path) - 9) || 'result.md')
) STRICT;

INSERT INTO artifacts_new SELECT * FROM artifacts WHERE kind = 'markdown';
DROP TABLE artifacts;
ALTER TABLE artifacts_new RENAME TO artifacts;

CREATE INDEX idx_conversions_active
    ON conversions(status)
    WHERE status IN ('queued', 'converting_local', 'finalizing');

CREATE INDEX idx_attempts_fifo
    ON attempts(queue_seq)
    WHERE state = 'queued';

COMMIT;

PRAGMA foreign_keys = ON;
