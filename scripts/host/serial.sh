#!/usr/bin/env bash
# Open a serial console to the board (115200 8N1 by default), and keep a log.
#
# The log is not a convenience. This project's rule is that a change is only
# `done (HW)` with a serial transcript in `docs/verification.md`, and until now
# that transcript depended on someone remembering to select the right lines out
# of a terminal scrollback before it wrapped. Every session is now recorded
# under `.serial-log/`, timestamped, whether or not anyone expected to need it —
# the boots worth keeping are usually the surprising ones, which are exactly
# the ones nobody prepared to capture.
#
# Only picocom can do this. minicom and screen are kept as fallbacks so the
# script still opens a console where picocom is absent, and both say plainly
# that they are not recording.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEV="${1:-/dev/ttyUSB0}"
BAUD="${2:-115200}"
LOG_DIR="${SERIAL_LOG_DIR:-${ROOT}/.serial-log}"

if [[ ! -e "${DEV}" ]]; then
	echo "error: serial device not found: ${DEV}" >&2
	echo "hint: plug a 3.3V USB-TTL adapter (GND, GPIO14 TX, GPIO15 RX)" >&2
	echo "hint: the adapter's RX goes to the Pi's TX (pin 8) and vice versa" >&2
	exit 1
fi

if command -v picocom >/dev/null 2>&1; then
	mkdir -p "${LOG_DIR}"
	LOG="${LOG_DIR}/$(date +%Y%m%d-%H%M%S).log"
	echo "recording to ${LOG}"
	echo "exit with C-a C-x"
	# `--imap lfcrlf` is for the display; the log keeps what the board sent.
	exec picocom -b "${BAUD}" --imap lfcrlf --logfile "${LOG}" "${DEV}"
fi

echo "warning: picocom not installed — this session will NOT be recorded" >&2
echo "         (pacman -S picocom), and a hardware claim needs a transcript" >&2

if command -v minicom >/dev/null 2>&1; then
	exec minicom -D "${DEV}" -b "${BAUD}"
fi
if command -v screen >/dev/null 2>&1; then
	exec screen "${DEV}" "${BAUD}"
fi

echo "error: install picocom, minicom, or screen" >&2
exit 1
