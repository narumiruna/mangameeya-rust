#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 1 || ! -d "$1" ]]; then
    printf 'Usage: bash packaging/upload-release.sh <artifact-directory>\n' >&2
    exit 1
fi
: "${GH_TOKEN:?GH_TOKEN is required}"
: "${GH_REPO:?GH_REPO is required}"
: "${RELEASES:?RELEASES is required}"
: "${RELEASE_COMMIT:?RELEASE_COMMIT is required}"
if [[ ! "$RELEASE_COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
    printf 'Expected a full release commit SHA.\n' >&2
    exit 1
fi
version="$(jq -er '
    if type == "array" and length == 1
        and .[0].package == "mangameeya-rust"
        and (.[0].version | type == "string" and test("\\A(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\z"))
        and .[0].tag == ("v" + .[0].version)
    then .[0].version else error("Expected one stable mangameeya-rust release") end
' <<< "$RELEASES")"
tag="v$version"
commit="$(git rev-parse --verify "refs/tags/$tag^{commit}")"
if [[ "$commit" != "$RELEASE_COMMIT" ]]; then
    printf 'Release tag does not point to the tested commit.\n' >&2
    exit 1
fi

files=("$1/MangaMeeya-Rust_${version}_windows-setup.exe")
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    for extension in zip dmg; do
        files+=("$1/MangaMeeya-Rust_${version}_${target}.$extension")
    done
done
# Validate the complete set before the first remote upload.
for file in "${files[@]}"; do
    if [[ ! -f "$file" || -L "$file" || ! -s "$file" ]]; then
        printf 'Missing, empty, or unsafe release package: %s\n' "$file" >&2
        exit 1
    fi
done
release="$(gh release view "$tag" --json tagName,isDraft,assets)"
jq -e --arg tag "$tag" '
    .tagName == $tag and .isDraft == false and (.assets | type == "array")
' <<< "$release" >/dev/null
# An interrupted GitHub upload can leave an empty starter asset. Do not hide
# that failure or replace it automatically, even if its name is correct.
for file in "${files[@]}"; do
    name="${file##*/}"
    if ! jq -e --arg name "$name" '
        all(.assets[] | select(.name == $name);
            .state == "uploaded" and (.size | type == "number" and . > 0))
    ' <<< "$release" >/dev/null; then
        printf 'Incomplete existing release asset; inspect it before retrying: %s\n' "$name" >&2
        exit 1
    fi
done
for file in "${files[@]}"; do
    name="${file##*/}"
    if jq -e --arg name "$name" 'any(.assets[]; .name == $name)' <<< "$release" >/dev/null; then
        printf 'Keep existing release asset: %s\n' "$name"
    else
        gh release upload "$tag" "$file"
    fi
done
