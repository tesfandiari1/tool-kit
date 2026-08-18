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

The fuller per-family matrix, including failure fixtures, is Increment 4
(CVR-036) and follows the rules in `backend/evals/README.md`.
