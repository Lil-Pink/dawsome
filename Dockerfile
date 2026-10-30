# The command line in a container: `dawsome render`, `smf-import`, `smf-export`,
# `export-midi`, `export-stems`, `export-dawproject` and `scan`, with the example projects (the demo among them). No window, no audio device.
#   docker build -t dawsome-cli .
#   docker run --rm -v "$PWD:/work" dawsome-cli render /usr/share/dawsome/examples/demo.json \
#       /work/demo.wav --song --bars 8
# scripts/docker-check.sh builds it and checks that its renders are the host's, byte for byte.

# The toolchain the repository is tested with, so a render is the same as the host's. Both
# images are pinned by digest as well as tag: a tag moves when its image is rebuilt, a digest
# names the same bytes for ever, so a later build of this file is this build. A new toolchain
# needs new digests (the registry's Docker-Content-Digest for the tag).
FROM docker.io/library/rust:1.93.1-trixie@sha256:ecbe59a8408895edd02d9ef422504b8501dd9fa1526de27a45b73406d734d659 AS build
# The time-stretch library's bindings are generated with libclang; LAME and the time-stretch
# library are built with the image's C and C++ compilers.
RUN apt-get update \
    && apt-get install -y --no-install-recommends clang libclang-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY examples examples
RUN cargo build --release --locked -p dawsome

FROM docker.io/library/debian:trixie-slim@sha256:62bd423aeee912639ca8ee681c2b762cb3e25137e0a92b20be455898a9f034ad
COPY --from=build /src/target/release/dawsome /usr/local/bin/dawsome
COPY --from=build /src/examples /usr/share/dawsome/examples
COPY LICENSE licenses/NOTICE licenses/THIRD_PARTY.md licenses/NOTICES.txt /usr/share/doc/dawsome/
# A user of its own: a render writes only where it is told to.
RUN useradd --create-home --uid 1000 dawsome
USER dawsome
WORKDIR /work
ENTRYPOINT ["dawsome"]
