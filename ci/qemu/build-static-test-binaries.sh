#!/usr/bin/env bash
#
# Build portable test executables on the GitHub runner.  The executables are
# copied into the cgroup VM and run there, avoiding a Rust build under QEMU
# software emulation.

set -euo pipefail

readonly TARGET="x86_64-unknown-linux-musl"
readonly OUTPUT_DIR="${1:?usage: build-static-test-binaries.sh OUTPUT_DIR}"

if [[ -e "${OUTPUT_DIR}" ]]; then
    echo "output directory already exists: ${OUTPUT_DIR}" >&2
    exit 1
fi

mkdir -p "${OUTPUT_DIR}"

messages="$(mktemp)"
executables="$(mktemp)"
manifest="$(mktemp)"
trap 'rm -f "${messages}" "${executables}" "${manifest}"' EXIT

cargo test \
    --all-features \
    --target "${TARGET}" \
    --no-run \
    --message-format=json >"${messages}"

jq -r '
    select(
        .reason == "compiler-artifact"
        and .profile.test == true
        and .executable != null
    )
    | .executable
' "${messages}" | sort -u >"${executables}"

if [[ ! -s "${executables}" ]]; then
    echo "cargo did not produce any test executables" >&2
    exit 1
fi

while IFS= read -r executable; do
    destination="${OUTPUT_DIR}/$(basename "${executable}")"
    cp "${executable}" "${destination}"
    strip "${destination}"

    if readelf -l "${destination}" | grep -q "Requesting program interpreter"; then
        echo "test executable is dynamically linked: ${destination}" >&2
        exit 1
    fi
done <"${executables}"

find "${OUTPUT_DIR}" -maxdepth 1 -type f -printf '%f\n' | sort >"${manifest}"
mv "${manifest}" "${OUTPUT_DIR}/manifest"

echo "Built $(wc -l <"${OUTPUT_DIR}/manifest") static test executables:"
sed 's/^/  /' "${OUTPUT_DIR}/manifest"
