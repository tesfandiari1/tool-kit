# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.97.1
ARG DEBIAN_VERSION=bookworm-20260623-slim

FROM rust:${RUST_VERSION}-bookworm AS builder
WORKDIR /build

# Least-churn first, so a source edit invalidates only the last COPY.
# `sqlx::migrate!("./migrations")` reads the SQL at compile time, so the
# migrations directory is a build input, not just a runtime asset.
COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY migrations ./migrations
COPY src ./src

RUN cargo build --locked --release --bins \
    && PDF_INSPECTOR_SOURCE="$(find /usr/local/cargo/registry/src -mindepth 2 -maxdepth 2 -type d -name 'pdf-inspector-1.15.0' -print -quit)" \
    && test -n "${PDF_INSPECTOR_SOURCE}" \
    && mkdir -p /build/pdf-inspector-runtime/licenses \
    && cp -R "${PDF_INSPECTOR_SOURCE}/external/bcmaps" /build/pdf-inspector-runtime/bcmaps \
    && cp "${PDF_INSPECTOR_SOURCE}/LICENSE" /build/pdf-inspector-runtime/licenses/pdf-inspector-MIT.txt \
    && cp "${PDF_INSPECTOR_SOURCE}/external/bcmaps/LICENSE" /build/pdf-inspector-runtime/licenses/adobe-bcmaps.txt

FROM debian:${DEBIAN_VERSION} AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 converter \
    && useradd --uid 10001 --gid converter --no-create-home --shell /usr/sbin/nologin converter

# A fresh named volume inherits the ownership of the image directory it covers,
# and falls back to root:root when the image has no such directory. The service
# runs as 10001, so the data root has to exist here or the first `compose up`
# cannot write. No VOLUME instruction: that would hand plain `docker run` an
# anonymous volume nobody prunes. Compose declares the volume instead.
RUN install -d -o 10001 -g 10001 -m 0700 /data

COPY --from=builder /build/target/release/tool-kit-converter /usr/local/bin/tool-kit-converter
COPY --from=builder /build/target/release/tool-kit-pdf-worker /usr/local/bin/tool-kit-pdf-worker
COPY --from=builder /build/pdf-inspector-runtime/bcmaps /opt/pdf-inspector/bcmaps
COPY --from=builder /build/pdf-inspector-runtime/licenses /usr/share/licenses/pdf-inspector

USER 10001:10001
EXPOSE 8080

# Direct image use stays loopback-only. Compose explicitly opts into the
# all-interface container listener while publishing only to host loopback.
ENV TOOLKIT_CONVERTER_BIND_ADDR=127.0.0.1:8080
ENV TOOLKIT_CONVERTER_PDF_BCMAPS_DIR=/opt/pdf-inspector/bcmaps
ENV RUST_LOG=tool_kit_converter=info

# Readiness probes SQLite, the data root, and the worker, so the start period has
# to cover migrations plus startup job reconciliation. `--start-interval` needs
# Docker 25 or newer.
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --start-interval=2s --retries=3 \
  CMD ["curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:8080/health/ready"]

ENTRYPOINT ["/usr/local/bin/tool-kit-converter"]
