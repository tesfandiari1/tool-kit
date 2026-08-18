-- no-transaction
-- Widen conversions.source_media_type to the rest of the AnyDoc format set:
-- OpenDocument (odt/ods/odp), RTF, CSV, and the macro-enabled and slideshow
-- OOXML variants. Every one of them has a passing round-trip fixture and a
-- bounded-failure fixture; `xlsb` is deliberately absent because no fixture
-- proves it. SQLite cannot ALTER a CHECK constraint, so both tables are
-- rebuilt and copied exactly as migration 0002 did. Foreign keys are disabled
-- for the rebuild and re-enabled after; the copy preserves rowids, so
-- attempts.queue_seq keeps its AUTOINCREMENT high-water mark.
--
-- `pps` and `pot` add no media type: they share `application/vnd.ms-powerpoint`
-- with `ppt` and differ only by extension.

PRAGMA foreign_keys = OFF;

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
                                      'application/epub+zip'
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

INSERT INTO conversions_new SELECT * FROM conversions;
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
                                      'needs_remote',
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
                                            'structured_document'
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

INSERT INTO attempts_new SELECT * FROM attempts;
DROP TABLE attempts;
ALTER TABLE attempts_new RENAME TO attempts;

CREATE INDEX idx_conversions_active
    ON conversions(status)
    WHERE status IN ('queued', 'converting_local', 'finalizing');

CREATE INDEX idx_attempts_fifo
    ON attempts(queue_seq)
    WHERE state = 'queued';

PRAGMA foreign_keys = ON;
