# Conversion evaluation corpus

The conversion service is corpus-gated. A parser or advertised format is not
accepted because one sample worked; it must pass labeled success and failure
fixtures representing the routing and completeness risks in this document.

## Data rules

- Use synthetic, public-domain, or explicitly licensed redistributable inputs.
- Never add customer, employer, school, legal, medical, or other private
  documents to the repository.
- Record source, license, redistribution permission, expected route, and
  expected outcome for every fixture.
- Store large or non-redistributable fixtures outside Git and reference them by
  SHA-256 through an approved local corpus path.
- Do not include document text, filenames, or extracted content in test logs.

## Minimum routing classes

- Clean native-text PDF
- Scanned PDF
- Image-based PDF
- Mixed native/image PDF
- Dense financial table PDF
- Multi-column PDF
- Form PDF
- Broken or garbled encoding
- Encrypted PDF
- Malformed and resource-limit input
- Each approved AnyDoc extension family
- Standalone image and HTML remote-route cases

`corpus-manifest.example.yaml` defines the initial metadata shape. M3 populates
the actual fixtures and expected results before M4 calibrates routing policy.
M8 freezes that completed corpus and captures old-path baseline outputs for the
release comparison.

