-- no-transaction
-- Drop the manifest artifact. A succeeded attempt publishes `result.md` alone.
--
-- The attempt records the Markdown's size and digest at finalizing, which is
-- what startup recovery checks a publication against before the success
-- commit. They are two new nullable columns, so `attempts` takes an ALTER.
--
-- `artifacts.kind` is a CHECK, so that table is rebuilt and copied without its
-- manifest rows. An old publication keeps its manifest.json on disk, and
-- validation tolerates it. Foreign keys are disabled for the rebuild and
-- re-enabled after.

PRAGMA foreign_keys = OFF;

-- The rebuild is one transaction. sqlx runs this file in autocommit, so
-- without it a kill between DROP and RENAME leaves an unbootable schema.
BEGIN;

ALTER TABLE attempts ADD COLUMN markdown_byte_length INTEGER
    CHECK(markdown_byte_length IS NULL OR markdown_byte_length > 0);
ALTER TABLE attempts ADD COLUMN markdown_sha256 TEXT
    CHECK(markdown_sha256 IS NULL OR (length(markdown_sha256) = 64
      AND markdown_sha256 NOT GLOB '*[^0-9a-f]*'));

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

COMMIT;

PRAGMA foreign_keys = ON;
