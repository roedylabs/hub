# Roedy Hub image (taken over from Roedy Mail's Dockerfile, ../mail). The
# Dockerfile only packages: web app and binary are built beforehand with
# `just dist` (in CI via ci.yml and release.yml) – statically linked musl
# binaries with the embedded SPA under dist/<amd64|arm64>/roedy-hub. Then
# `docker buildx build --platform linux/amd64,linux/arm64 .` builds without
# emulation, because no stage runs code of the target platform. The example app
# (roedy-hub-example) is not part of the image.
#
# No `# syntax=` line on purpose: the Dockerfile frontend built into BuildKit
# is used, and the workflows run BuildKit pinned by digest
# (.github/buildkit/Dockerfile). Every image is pinned by tag and digest, which
# Dependabot keeps up to date; scripts/docker-pins.test.mjs checks that.

# File system for scratch: CA certificates (apps with an `https` internal
# address, system mails over JMAP), /data for USER 65532, /tmp for SQLite.
FROM --platform=$BUILDPLATFORM alpine:3.24@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf AS rootfs
RUN apk add --no-cache ca-certificates \
 && mkdir -p /rootfs/etc/ssl/certs /rootfs/data /rootfs/tmp \
 && cp /etc/ssl/certs/ca-certificates.crt /rootfs/etc/ssl/certs/ \
 && chown 65532:65532 /rootfs/data \
 && chmod 0700 /rootfs/data \
 && chmod 1777 /rootfs/tmp

FROM scratch
ARG TARGETARCH
LABEL org.opencontainers.image.title="Roedy Hub" \
      org.opencontainers.image.description="Gateway, sign-in and home page for the Roedy apps" \
      org.opencontainers.image.url="https://github.com/roedylabs/hub" \
      org.opencontainers.image.source="https://github.com/roedylabs/hub" \
      org.opencontainers.image.documentation="https://github.com/roedylabs/hub/blob/main/docs/ops.md" \
      org.opencontainers.image.licenses="AGPL-3.0-only"
COPY --from=rootfs /rootfs/ /
COPY --chmod=0755 dist/${TARGETARCH}/roedy-hub /usr/local/bin/roedy-hub
COPY LICENSE /usr/share/doc/roedy-hub/LICENSE
# mimalloc returns free memory immediately and uses no Transparent Huge Pages:
# otherwise the idle server keeps what startup and sign-ins (Argon2, 19 MiB per
# password) allocated.
ENV PATH=/usr/local/bin \
    ROEDY_DATA_DIR=/data \
    MIMALLOC_PURGE_DELAY=0 \
    MIMALLOC_ALLOW_THP=0
USER 65532:65532
VOLUME /data
# Only the public ports: 8080 is HTTP (with ROEDY_TLS=off the hub itself,
# with TLS on ACME HTTP-01 and the redirect to HTTPS), 8443 is HTTPS with TLS
# on (docs/ops.md › TLS). The internal listener (ROEDY_INTERNAL_LISTEN, default
# 0.0.0.0:8081) is for the apps on the private network and is deliberately not
# exposed, so that `docker run -P` never publishes it (docs/ops.md).
EXPOSE 8080 8443
# With ROEDY_TLS=acme the check fails until the first certificate exists,
# usually within a minute: a start period of 3 minutes, checked every second
# within it, so a container is healthy as soon as it serves.
HEALTHCHECK --interval=30s --timeout=5s --start-period=3m --start-interval=1s --retries=3 \
  CMD ["roedy-hub", "healthcheck"]
ENTRYPOINT ["roedy-hub"]
CMD ["serve"]
