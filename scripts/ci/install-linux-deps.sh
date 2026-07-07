#!/usr/bin/env bash
# Retry apt installs — GitHub ubuntu-latest runners often hit exit 100 (dpkg lock / mirror flake).
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: install-linux-deps.sh <pkg> [pkg...]"
  exit 1
fi

export DEBIAN_FRONTEND=noninteractive
PACKAGES=("$@")

for attempt in 1 2 3 4 5; do
  echo "apt install attempt ${attempt}/5: ${PACKAGES[*]}"
  if sudo apt-get update -o Acquire::Retries=3 \
    && sudo apt-get install -y --no-install-recommends "${PACKAGES[@]}"; then
    echo "apt install succeeded"
    exit 0
  fi
  echo "apt failed; waiting $((attempt * 15))s before retry..."
  sleep $((attempt * 15))
done

echo "::error::apt install failed after 5 attempts"
exit 1
