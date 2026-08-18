# Fixture sources

Fixtures here are vendored from the upstream AnyDoc test corpus,
[`firecrawl/anydoc`](https://github.com/firecrawl/anydoc) `tests/fixtures/`,
MIT licensed. AnyDoc is the engine under test, so its own fixtures are the
honest baseline: if the service cannot convert what the engine's authors test
with, the adapter is broken.

| File | Upstream path | License |
|---|---|---|
| `anydoc/text.docx` | `tests/fixtures/docx/text.docx` | MIT |
| `anydoc/sheet.xlsx` | `tests/fixtures/xlsx/sheet.xlsx` | MIT |
| `anydoc/pres.pptx` | `tests/fixtures/pptx/pres.pptx` | MIT |
| `anydoc/text.doc` | `tests/fixtures/doc/text.doc` | MIT |
| `anydoc/sheet.xls` | `tests/fixtures/xls/sheet.xls` | MIT |
| `anydoc/handmade-multimaster.ppt` | `tests/fixtures/ppt/handmade-multimaster.ppt` | MIT |
| `anydoc/book.epub` | `tests/fixtures/epub/book.epub` | MIT |
| `anydoc/truncated.docx` | `tests/fixtures/malformed/truncated--errors.docx` | MIT |
| `anydoc/truncated.doc` | `tests/fixtures/malformed/truncated--errors.doc` | MIT |
| `anydoc/deepnest.ppt` | `tests/fixtures/abuse/deepnest--errors.ppt` | MIT |
| `anydoc/hugespan.pptx` | `tests/fixtures/abuse/hugespan--errors.pptx` | MIT |
| `anydoc/truncated.xls` | derived: first 2000 bytes of `sheet.xls` | MIT |
| `anydoc/truncated.xlsx` | derived: first 2000 bytes of `sheet.xlsx` | MIT |
| `anydoc/truncated.epub` | derived: first 2000 bytes of `book.epub` | MIT |

The fuller per-family matrix, including failure fixtures, is Increment 4
(CVR-036) and follows the rules in `backend/evals/README.md`.
