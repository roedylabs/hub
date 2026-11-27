# Image of the example app (crates/example-app) for deploy/compose.test.yml and
# the end-to-end tests; never published. Like the hub's Dockerfile it only
# packages the static binary built beforehand with `just dist-example`
# (dist/<amd64|arm64>/roedy-hub-example). Build context is the repository root:
#
#   docker buildx build --load -f deploy/example-app.Dockerfile -t roedy-hub-example:dev .
#
# The example keeps its pairing in memory only and needs no data directory.

# CA certificates: the example's HTTP client (rustls with the platform
# verifier) refuses to start without any.
FROM --platform=$BUILDPLATFORM alpine:3.24@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf AS rootfs
RUN apk add --no-cache ca-certificates \
 && mkdir -p /rootfs/etc/ssl/certs \
 && cp /etc/ssl/certs/ca-certificates.crt /rootfs/etc/ssl/certs/

FROM scratch
ARG TARGETARCH
LABEL org.opencontainers.image.title="Roedy Hub example app" \
      org.opencontainers.image.description="Example app implementing the Roedy Hub app contract, for tests" \
      org.opencontainers.image.source="https://github.com/roedylabs/hub" \
      org.opencontainers.image.licenses="AGPL-3.0-only"
COPY --from=rootfs /rootfs/ /
COPY --chmod=0755 dist/${TARGETARCH}/roedy-hub-example /usr/local/bin/roedy-hub-example
COPY LICENSE /usr/share/doc/roedy-hub-example/LICENSE
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/roedy-hub-example"]
