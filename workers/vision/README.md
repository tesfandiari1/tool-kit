# Vision worker

`main.swift` is `tool-kit-vision-worker`, the converter's Vision engine. It
reads an image or a scanned PDF into Markdown, and describes DOCX and PPTX
pictures through `--describe`. Its wire contract is
`crates/worker-protocol/src/vision.rs`, and `pnpm sidecars` builds and stages it.

`pdf2png` rasterizes a PDF into a realistic scan. The `vision_worker` tests
use it to build their scan fixture.

`swiftc` ships with the Command Line Tools, so there is no Xcode, no SwiftPM,
no dependency, and no lockfile. `bin/` is gitignored.

```bash
./build.sh
bin/pdf2png <in.pdf> <out.png> [dpi]     # make a realistic scan out of a text PDF
```

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

**The collections overlap.** `title`, `paragraphs`, `lists`, and `tables` each
report the same text, so a renderer that walks all four prints every sentence
two or three times. `markdown(_:)` claims what it has emitted and skips it.

**Reading order comes from geometry, not from the API.** The collections are
not one ordered stream, so blocks are sorted by the top edge of their bounding
region. Vision normalizes with the origin at the bottom left, so descending
order is reading order.

**Speed:** 386 to 1047 ms per page at 200 dpi, single threaded, untuned.

## Long scans

**One CGPDFDocument per page.** A document keeps every decoded page image
until it is freed, about 40 MB a page on an Internet Archive scan. One document
for a 632-page book grew past 16 GB. `renderPage(_:of:dpi:)` opens a document
per page from the shared `CGDataProvider`, at about 2 ms each. An
`autoreleasepool` does not help, and reopening every 10 pages still held 1.2 GB.

**Render and read overlap.** The next page renders while Vision reads the
current one, so at most two bitmaps are held. On a 30-page MRC scan this took
the run from 20.4 s to 12.8 s with byte-identical output.

**Progress.** After each PDF page the worker writes `<done> <total>\n` to
`vision-progress` in its cwd, through a rename. Every 25 pages and on the last
it prints `tool-kit-vision-worker: page <n>/<total> footprint=<MB>MB` to stderr.

Not taken: OCR on the 1-bit JBIG2 text mask alone. It was faster but matched
the full render on only 96% of words, with e-to-c errors and lost headings.

## Corpus note

`apps/converter/tests/support/corpus.rs` generates its `scanned` fixture as a coarse
checkerboard. It exists to make `pdf-inspector` classify a page image-only, and
it carries no text, so Vision correctly returns nothing from it. Judging OCR
needs a fixture that holds real words: use `pdf2png` on the `native` fixture,
or add a rendered-text image fixture.
