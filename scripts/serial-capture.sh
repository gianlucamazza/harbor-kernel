#!/usr/bin/env bash
# Record the board's UART to a timestamped log, without holding a terminal.
#
# `serial.sh` opens an interactive console and is the right tool when a human
# is driving. This is the other half: it waits for the adapter, records every
# line with the host clock beside it, and exposes a FIFO so bytes can be sent
# *to* the board from a script.
#
# Both halves exist because of two things that happened in one session:
#
#   - The board rebooted itself after `*** halt ***`, and the log could not say
#     how long afterwards, because picocom's transcript carries no time. Any
#     question of the form "how long between X and Y" was unanswerable.
#   - The PL011 RX handover needed a byte to arrive inside a window a couple of
#     instructions wide. A hand cannot hit a window it cannot see; a loop
#     writing to a FIFO can.
#
# Timestamps are the *host's*, and say when a line arrived at this end of the
# wire — not when the board emitted it. At 115200 with short lines the two are
# within a millisecond, which is far below anything worth reasoning about here.
set -euo pipefail

# `EPOCHREALTIME` renders its fractional part with the locale's radix
# character, so on an it_IT machine it reads `1786026450,255472` and the
# `${...#*.}` below strips nothing — every timestamp came out as
# `16:27:30.1786026450,255472`. Pinning the numeric locale is the fix at the
# cause; matching both separators downstream would only paper over it.
export LC_NUMERIC=C

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BAUD="${BAUD:-115200}"
LOG_DIR="${SERIAL_LOG_DIR:-${ROOT}/.serial-log}"
WAIT_SECONDS="${WAIT_SECONDS:-0}"

# `EPOCHREALTIME` is a bash 5 variable and costs no fork per line, which a
# `date` call would. At 115200 baud a fork per line is affordable, but this
# runs for the whole of a hardware session and there is no reason to pay it.
if [[ -z "${EPOCHREALTIME:-}" ]]; then
	echo "error: bash 5 or newer required (EPOCHREALTIME)" >&2
	exit 1
fi

find_adapter() {
	local candidate
	for candidate in /dev/ttyUSB* /dev/ttyACM*; do
		[[ -e "${candidate}" ]] && printf '%s' "${candidate}" && return 0
	done
	return 1
}

DEV="${1:-}"
if [[ -z "${DEV}" ]]; then
	waited=0
	until DEV="$(find_adapter)"; do
		if [[ "${WAIT_SECONDS}" -eq 0 ]]; then
			echo "error: no serial adapter (/dev/ttyUSB* or /dev/ttyACM*)" >&2
			echo "hint: plug a 3.3V USB-TTL adapter — its RX to the Pi's TX (pin 8)," >&2
			echo "      its TX to the Pi's RX (pin 10), GND to pin 6, no 5V" >&2
			echo "hint: or set WAIT_SECONDS to wait for one to appear" >&2
			exit 1
		fi
		[[ "${waited}" -ge "${WAIT_SECONDS}" ]] && {
			echo "error: no serial adapter after ${WAIT_SECONDS}s" >&2
			exit 1
		}
		sleep 1
		waited=$((waited + 1))
	done
fi

# Raw 8N1, no flow control, no local echo. `-hupcl` keeps the line from being
# dropped when a reader exits, so restarting a capture does not toggle the
# adapter's control lines under a board that is mid-boot.
stty -F "${DEV}" "${BAUD}" cs8 -cstopb -parenb -crtscts -ixon -ixoff -echo -hupcl raw

mkdir -p "${LOG_DIR}"
LOG="${LOG_DIR}/$(date +%Y%m%d-%H%M%S).log"
FIFO="${LOG_DIR}/tx.fifo"

[[ -p "${FIFO}" ]] || {
	rm -f "${FIFO}"
	mkfifo "${FIFO}"
}

# Hold the FIFO open from this shell. Without a reader that stays open, every
# `printf > tx.fifo` would see the previous writer's EOF close the pipe, and
# the transmit path would work for one byte and then stop.
exec 3<>"${FIFO}"
cat <&3 >"${DEV}" &
tx_pid=$!
trap 'kill "${tx_pid}" 2>/dev/null || true; exec 3>&-' EXIT

echo "capturing ${DEV} @ ${BAUD} 8N1"
echo "log:  ${LOG}"
echo "send: printf 'x' > ${FIFO}"
echo "---"

# `tr -d '\r'` because the board sends CRLF, and a bare CR makes the log
# unreadable in anything that respects it. The timestamp is prepended per line;
# a line the board never terminated is flushed when the next one starts, which
# is the only case where the recorded time lags the arrival.
stdbuf -o0 tr -d '\r' <"${DEV}" | while IFS= read -r line; do
	printf '%(%H:%M:%S)T.%s %s\n' -1 "${EPOCHREALTIME#*.}" "${line}"
done | tee -a "${LOG}"
