# Third-party runtime notices

The M1 container includes `pdf-inspector` 1.15.0 under the MIT License and its
bundled Adobe CMap resources under their separate BSD-style notice. The image
stores the complete notices at:

```text
/usr/share/licenses/pdf-inspector/pdf-inspector-MIT.txt
/usr/share/licenses/pdf-inspector/adobe-bcmaps.txt
```

The dependency is compiled with default features disabled. The runtime image
does not include the optional OCR, rendering, model-cache, or model-download
stack.

This file is not yet the complete transitive Rust license inventory. M1 is a
development-only image. CVR-076 requires an SBOM, full transitive notices, and a
current advisory review before distribution or LAN deployment; that review must
also resolve or explicitly block on the unmaintained `ttf-parser 0.25.1`
advisory RUSTSEC-2026-0192.
