#!/bin/sh
set -eu

crate_dir="${1:-.}"

cd "$crate_dir"
cargo build --release --locked
mkdir -p bin
cp target/release/scope bin/scope
cp target/release/scope-cli-service bin/scope-cli-service
