#!/bin/sh
set -eu

# Candidate validation trigger: the package source tree is frozen; this script-only
# comment forces the path-filtered build workflow to validate the exact review branch.
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
