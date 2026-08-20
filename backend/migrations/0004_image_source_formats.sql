-- no-transaction
-- Widen conversions.source_media_type to the six raster image media types the
-- Vision engine OCRs (png, jpeg, webp, tiff, gif, bmp), and record the two OCR
-- settings a caller may send with an image. SQLite cannot ALTER a CHECK
-- constraint, so the table is rebuilt and copied as migrations 0002 and 0003
-- did. Foreign keys are disabled for the rebuild and re-enabled after.
--
-- Only `conversions` changes, so only `idx_conversions_active` is recreated:
-- an index dies with its table and recreating a surviving one is an error.
--
-- The OCR settings live on the conversion because the runner claims work from
-- this table long after the request is gone, and they must reach the worker
-- unchanged across a restart. They mean nothing to the PDF and AnyDoc engines,
-- which is why their defaults are the copy values for every existing row.

PRAGMA foreign_keys = OFF;

-- The rebuild is one transaction. sqlx runs this file in autocommit, so
-- without it a kill between DROP and RENAME leaves an unbootable schema.
BEGIN;

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
    profile               TEXT NOT NULL
                                  CHECK(profile IN (
                                      'standard',
                                      'local_only',
                                      'best_quality'
                                  )),
    status                TEXT NOT NULL
                                  CHECK(status IN (
                                      'queued',
                                      'converting_local',
                                      'finalizing',
                                      'succeeded',
                                      'failed',
                                      'needs_remote'
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
                                      'image/bmp'
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

-- Positional, as in 0003. The two trailing literals are the OCR defaults for
-- rows written before this migration existed.
INSERT INTO conversions_new SELECT *, 1, '' FROM conversions;
DROP TABLE conversions;
ALTER TABLE conversions_new RENAME TO conversions;

CREATE INDEX idx_conversions_active
    ON conversions(status)
    WHERE status IN ('queued', 'converting_local', 'finalizing');

COMMIT;

PRAGMA foreign_keys = ON;
