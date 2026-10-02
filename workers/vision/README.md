# Vision worker and tools

`main.swift` is `tool-kit-vision-worker`, the converter's Vision engine. It
reads an image or a scanned PDF into Markdown, and describes DOCX and PPTX
pictures through `--describe`. Its wire contract is
`crates/worker-protocol/src/vision.rs`, and `pnpm sidecars` builds and stages it.

`tk-vision` and `pdf2png` are development tools for judging Apple Vision's
output on real documents. Nothing in the converter calls them.

`swiftc` ships with the Command Line Tools, so there is no Xcode, no SwiftPM,
no dependency, and no lockfile. `bin/` is gitignored.

```bash
./build.sh
bin/tk-vision <file.pdf|file.png> [--flat] [--page N] [--dpi N] [--fast]
bin/pdf2png <in.pdf> <out.png> [dpi]     # make a realistic scan out of a text PDF
```

`--flat` prints `document.text.transcript`, the ungrouped baseline, so the
structured output can be judged against what plain OCR would have given.

## What the first measurements found

**macOS 26 is a hard floor.** `RecognizeDocumentsRequest` arrived in macOS 26.
`build.sh` refuses below it rather than failing at runtime.

**Transcription is strong.** A rendered page of known text came back 24 of 24
lines byte-perfect. A 2015 scanned government letter came back with roughly six
errors in 450 words, correct reading order, and its headings and numbered list
intact.

**Table detection is unreliable in both directions, so the renderer checks its
work.** On a real form of labeled fields it reported zero tables and returned
65 loose paragraphs with no label-to-value relationship. On a synthetic page of
24 evenly spaced identical lines it reported a false 12-column table with 11
columns empty. Paragraphs, lists, headings, and reading order are the parts
that hold up.

`prune` in `render.swift` answers the over-reaching half: a column empty in
every row is not a column, and a table with one column left is prose, not a
grid. A real table has content in most of its columns and survives untouched.
`evenly_spaced_lines_publish_as_prose_rather_than_a_false_table` in
`apps/converter/tests/vision_worker.rs` pins it. Nothing answers the under-reaching
half, so a form still arrives as loose fields.

**`render.swift` is compiled into both binaries** so the preview tool cannot
drift from what the worker publishes. It drifted once already: the false-table
guard landed in the worker while `tk-vision` kept its own copy and went on
printing the table the worker had stopped emitting.

**The collections overlap.** `title`, `paragraphs`, `lists`, and `tables` each
report the same text, so a renderer that walks all four prints every sentence
two or three times. `markdown(_:)` claims what it has emitted and skips it.

**Reading order comes from geometry, not from the API.** The collections are
not one ordered stream, so blocks are sorted by the top edge of their bounding
region. Vision normalizes with the origin at the bottom left, so descending
order is reading order.

**Speed:** 386 to 1047 ms per page at 200 dpi, single threaded, untuned.

## Corpus note

`apps/converter/tests/support/corpus.rs` generates its `scanned` fixture as a coarse
checkerboard. It exists to make `pdf-inspector` classify a page image-only, and
it carries no text, so Vision correctly returns nothing from it. Judging OCR
needs a fixture that holds real words: use `pdf2png` on the `native` fixture,
or add a rendered-text image fixture.
