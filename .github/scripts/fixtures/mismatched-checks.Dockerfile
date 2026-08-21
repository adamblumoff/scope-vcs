FROM --platform=linux/amd64 rust:1.98.0-bookworm@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa AS builder
FROM --platform=linux/amd64 rust:1.97.0-bookworm@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
ENV RUSTUP_TOOLCHAIN=1.98.0-x86_64-unknown-linux-gnu
RUN test "$(rustc --version | cut -d' ' -f2)" = "1.98.0"
