#!/usr/bin/env bash
#
# Run the compiled Rust test harnesses inside a cgroup test VM.

set -euo pipefail

readonly TEST_DIR="${1:?usage: run-test-binaries-in-guest.sh TEST_DIR}"

cd "${TEST_DIR}"

cleanup_failed_test() {
    # A failed test can leave a spawned helper holding the SSH channel open.
    # The VM is disposable, so terminate only the helper commands used by
    # this suite before returning.
    sudo pkill -KILL -x sleep || true
    sudo pkill -KILL -x yes || true
}

run_test() {
    local test_binary="$1"
    shift

    echo
    echo "Running ${test_binary} $*"

    local result=0
    sudo -E "./${test_binary}" "$@" --color always --nocapture \
        --test-threads=1 || result=$?
    if ((result != 0)); then
        cleanup_failed_test
        exit "${result}"
    fi
}

library_test="$(grep -E "^cgroups_rs-" manifest)"
if [[ "$(wc -l <<<"${library_test}")" -ne 1 ]]; then
    echo "expected exactly one library test executable" >&2
    exit 1
fi

# Match the phases in the Makefile: cgroup-manipulating suites are isolated
# and sequential, followed by all remaining unit tests.
run_test "${library_test}" systemd::dbus::client::tests
run_test "${library_test}" manager::fs::tests
run_test "${library_test}" manager::systemd::tests
run_test "${library_test}" \
    --skip systemd::dbus::client::tests \
    --skip manager::fs::tests \
    --skip manager::systemd::tests

while IFS= read -r test_binary; do
    [[ "${test_binary}" == "${library_test}" ]] && continue
    run_test "${test_binary}"
done < manifest
