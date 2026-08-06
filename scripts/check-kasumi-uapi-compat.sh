#!/bin/sh
set -eu

CC_BIN="${CC:-cc}"
TMP_BIN="$(mktemp "${TMPDIR:-/tmp}/kasumi-uapi-compat.XXXXXX")"
trap 'rm -f "$TMP_BIN"' EXIT HUP INT TERM

"$CC_BIN" \
    -std=c11 \
    -Wall \
    -Wextra \
    -Werror \
    -I. \
    tests/kasumi_uapi_compat.c \
    -o "$TMP_BIN"

"$TMP_BIN"
echo "Kasumi protocol 16/17 ABI compatibility check passed."
