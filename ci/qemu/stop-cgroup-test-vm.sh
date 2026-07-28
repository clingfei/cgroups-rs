#!/usr/bin/env bash
#
# Stop a cgroup test VM started by boot-cgroup-test-vm.sh.

set -euo pipefail

readonly STATE_DIR="${1:?usage: stop-cgroup-test-vm.sh STATE_DIR}"
readonly PID_FILE="${STATE_DIR}/qemu.pid"
readonly DISK="${STATE_DIR}/root.qcow2"

if [[ ! -f "${PID_FILE}" ]]; then
    exit 0
fi

pid="$(<"${PID_FILE}")"
if ! kill -0 "${pid}" 2>/dev/null; then
    rm -f -- "${PID_FILE}"
    exit 0
fi

if [[ ! -r "/proc/${pid}/cmdline" ]]; then
    echo "cannot verify process ${pid} from ${PID_FILE}" >&2
    exit 1
fi

cmdline="$(tr '\0' '\n' <"/proc/${pid}/cmdline")"
if [[ "${cmdline}" != *"${DISK}"* ]]; then
    echo "refusing to stop unrelated process ${pid} from stale ${PID_FILE}" >&2
    rm -f -- "${PID_FILE}"
    exit 0
fi

kill "${pid}"

for _ in {1..20}; do
    if ! kill -0 "${pid}" 2>/dev/null; then
        rm -f -- "${PID_FILE}"
        exit 0
    fi
    sleep 1
done

kill -KILL "${pid}" 2>/dev/null || true
for _ in {1..20}; do
    if ! kill -0 "${pid}" 2>/dev/null; then
        rm -f -- "${PID_FILE}"
        exit 0
    fi
    sleep 1
done

echo "QEMU process ${pid} did not stop" >&2
exit 1
