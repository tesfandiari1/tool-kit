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
| `anydoc/text.odt` | `tests/fixtures/odt/text.odt` | MIT |
| `anydoc/sheet.ods` | `tests/fixtures/ods/sheet.ods` | MIT |
| `anydoc/pres.odp` | `tests/fixtures/odp/pres.odp` | MIT |
| `anydoc/text.rtf` | `tests/fixtures/rtf/text.rtf` | MIT |
| `anydoc/sheet.csv` | `tests/fixtures/csv/sheet.csv` | MIT |
| `anydoc/encrypted.odt` | `tests/fixtures/malformed/encrypted--errors.odt` | MIT |
| `anydoc/hugerepeat.ods` | `tests/fixtures/abuse/hugerepeat--errors.ods` | MIT |
| `anydoc/truncated.odp` | derived: first 2000 bytes of `pres.odp` | MIT |
| `anydoc/empty.rtf` | derived: a bare `{\rtf1` group | MIT |
| `anydoc/empty.csv` | derived: blank lines only | MIT |
| `anydoc/text.docm` | derived from `text.docx`: main-part content type rewritten to the macro-enabled type plus a `vbaProject.bin` | MIT |
| `anydoc/sheet.xlsm` | derived from `sheet.xlsx`, same rewrite | MIT |
| `anydoc/pres.pptm` | derived from `pres.pptx`, same rewrite | MIT |
| `anydoc/pres.ppsm` | derived from `pres.pptx`, slideshow macro-enabled type | MIT |
| `anydoc/pres.ppsx` | derived from `pres.pptx`, slideshow type | MIT |
| `anydoc/deck.pps` | copy of `handmade-multimaster.ppt`; `pps` is the same OLE container | MIT |
| `anydoc/deck.pot` | copy of `handmade-multimaster.ppt`; `pot` is the same OLE container | MIT |
| `anydoc/truncated.epub` | derived: first 2000 bytes of `book.epub` | MIT |

Every advertised extension has both a round-trip fixture and a bounded-failure
fixture here, asserted by `every_advertised_anydoc_family_converts` and
`broken_and_hostile_anydoc_inputs_fail_closed_without_artifacts`.

The macro-enabled fixtures are derived rather than vendored because the upstream
corpus has none. Each is a real OPC package: the main part's content type is
rewritten to the macro-enabled type and a `vbaProject.bin` is added, so
detection resolves them the way a genuine `.docm` resolves. That matters,
because `.docm` and `.pptm` content types contain `ms-word` / `ms-powerpoint`
rather than `wordprocessingml` / `presentationml`, so `opc_format` returns
`None` and the main part's root element is what identifies them.

`xlsb` is **not** advertised. AnyDoc maps the extension to its Excel parser and
`calamine` compiles with binary-workbook support, but no upstream fixture exists
and a binary workbook cannot be honestly synthesised from an XML one. It stays
out until a real file proves it.
