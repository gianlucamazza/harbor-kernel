#!/usr/bin/env bash
# Open a serial console to the board (115200 8N1 by default).
set -euo pipefail

DEV="${1:-/dev/ttyUSB0}"
BAUD="${2:-115200}"

if [[ ! -e "${DEV}" ]]; then
  echo "error: serial device not found: ${DEV}" >&2
  echo "hint: plug a 3.3V USB-TTL adapter (GND, GPIO14 TX, GPIO15 RX)" >&2
  exit 1
fi

if command -v picocom >/dev/null 2>&1; then
  exec picocom -b "${BAUD}" --imap lfcrlf "${DEV}"
fi
if command -v minicom >/dev/null 2>&1; then
  exec minicom -D "${DEV}" -b "${BAUD}"
fi
if command -v screen >/dev/null 2>&1; then
  exec screen "${DEV}" "${BAUD}"
fi

echo "error: install picocom, minicom, or screen" >&2
exit 1
