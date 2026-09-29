#!/usr/bin/env bash
# Fail when a Linux binary needs a newer glibc than the supported floor.
#
# usage: check-glibc-floor.sh <binary> <floor, e.g. 2.28>
#
# This reads the version needs (.gnu.version_r), not the symbols: the loader
# refuses to start a binary with an unmet version need even when every symbol
# using that version is weak, which is how v3.0.3 came to need glibc 2.39.
set -euo pipefail

binary="$1"
floor="$2"

needed=$(readelf -V --wide "$binary" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -uV | tail -n 1)
if [ -z "$needed" ]; then
  echo "::error title=glibc floor::found no GLIBC version needs in ${binary}"
  exit 1
fi

highest=$(printf '%s\n%s\n' "$needed" "$floor" | sort -V | tail -n 1)
if [ "$highest" != "$floor" ]; then
  echo "::error title=glibc floor::${binary} needs glibc ${needed}, above the supported floor ${floor}."
  exit 1
fi

echo "${binary} needs glibc ${needed} (floor ${floor})."
