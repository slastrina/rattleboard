# syntax=docker/dockerfile:1.7
# Rattleboard Web. Build from the repository root:
#   docker build -f crates/rattleboard-web/Dockerfile -t rattleboard-web .
# Cross-compiles on the build machine for each target platform (amd64, arm64) into one
# static binary, then ships it on a distroless base (CA certificates, time zones, no shell).

FROM --platform=$BUILDPLATFORM tonistiigi/xx:1.6.1 AS xx

FROM --platform=$BUILDPLATFORM rust:1-alpine AS build
COPY --from=xx / /
RUN apk add --no-cache clang lld musl-dev file
ARG TARGETPLATFORM
RUN xx-apk add --no-cache musl-dev gcc
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=rattleboard-web-target-$TARGETPLATFORM \
    xx-cargo build --release --locked -p rattleboard-web \
 && t=target/$(xx-cargo --print-target-triple)/release/rattleboard-web \
 && xx-verify --static "$t" \
 && cp "$t" /rattleboard-web \
 && mkdir -p /empty/data

FROM gcr.io/distroless/static-debian12:nonroot
LABEL org.opencontainers.image.title="Rattleboard Web" \
      org.opencontainers.image.description="A self-hosted split-flap board for any browser, with the Rattleboard API" \
      org.opencontainers.image.source="https://github.com/slastrina/rattleboard" \
      org.opencontainers.image.licenses="MIT"
COPY --from=build /rattleboard-web /usr/local/bin/rattleboard-web
# Owned by the image's user, so a new named volume is writable.
COPY --from=build --chown=65532:65532 /empty/data /data
ENV RATTLEBOARD_DATA=/data RATTLEBOARD_PORT=7070
VOLUME /data
EXPOSE 7070
USER nonroot
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s CMD ["/usr/local/bin/rattleboard-web", "health"]
ENTRYPOINT ["/usr/local/bin/rattleboard-web"]
