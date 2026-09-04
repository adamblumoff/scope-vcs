#!/bin/sh
set -eu

set -- ./bin/*
if [ "$#" -ne 1 ] || [ ! -x "$1" ]; then
  echo "Expected exactly one executable in ./bin." >&2
  exit 1
fi

exec "$1"
