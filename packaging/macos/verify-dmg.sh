#!/bin/bash
set -euo pipefail

if [[ $# -ne 1 || ! -f "$1" ]]; then
    printf 'Usage: bash packaging/macos/verify-dmg.sh <image.dmg>\n' >&2
    exit 1
fi

# macOS can hold a newly created image briefly. Retry only that transient error,
# never checksum failures or other verification errors.
for attempt in 1 2 3; do
    if output="$(LC_ALL=C hdiutil verify "$1" </dev/null 2>&1)"; then
        printf '%s\n' "$output"
        exit 0
    else
        status=$?
    fi
    printf '%s\n' "$output" >&2
    if [[ $attempt -eq 3 || "$output" != *'Resource temporarily unavailable'* ]]; then
        exit "$status"
    fi
    printf 'DMG verification temporarily unavailable; retrying (%s/3) in 2 seconds.\n' "$((attempt + 1))" >&2
    sleep 2
done
