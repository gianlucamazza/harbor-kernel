#!/usr/bin/env bash
# Device-window count the boot and product oracles assert.
#
# One fact, one owner: `src/bootstrap/authority.rs` declares the windows
# (`WINDOW_RNG` = 0, `WINDOW_FRAMEBUFFER` = 1 after ADR-0101/0113). Both
# oracles used to hard-code `N` in `authority: windows N declared`, and the
# copies drifted — product-oracle was updated for the framebuffer window,
# boot-oracle was not. Deriving N from the kernel constants means they cannot
# disagree with each other or invent a third table.
#
# `make vocabulary-sync` still compares those same constants to pack-store.py.
# This file does not replace that gate; it stops the two *oracles* from being
# a second, silent copy of the count.
#
# Sourced by `boot-oracle.sh` and `product-oracle.sh`. Idempotent: the hardware
# transcript check sources both, so this must survive a second include.

if [[ -n "${WINDOW_VOCAB_SOURCED:-}" ]]; then
	return 0
fi
WINDOW_VOCAB_SOURCED=1

_window_vocab_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
_window_vocab_authority="${_window_vocab_root}/src/bootstrap/authority.rs"
_window_vocab_loader="${_window_vocab_root}/src/bootstrap/loader.rs"

if [[ ! -f "${_window_vocab_authority}" ]]; then
	echo "window-vocab: ${_window_vocab_authority} is missing" >&2
	return 2
fi

# Index constants only (`WINDOW_RNG: u8 = 0`), not the parallel WINDOW_NAME_*
# strings. The printed count is windows.len() after one declare_window per
# index; that is this tally.
# Exported: consumed by boot-oracle.sh and product-oracle.sh after they source us.
export WINDOW_VOCAB_LEN
WINDOW_VOCAB_LEN="$(
	awk '/^pub const WINDOW_[A-Z0-9_]+: u8 = [0-9]+;/ { n++ } END { print n+0 }' \
		"${_window_vocab_authority}"
)"

if [[ "${WINDOW_VOCAB_LEN}" -lt 1 ]]; then
	echo "window-vocab: parsed no WINDOW_* index from ${_window_vocab_authority}" >&2
	return 2
fi

# Oracle-only `nowindow` names an index past the vocabulary (ADR-0100), so the
# arithmetic refusal is on every good boot. The index lives next to the entry
# in loader.rs; the "of N" half is WINDOW_VOCAB_LEN.
if [[ -f "${_window_vocab_loader}" ]]; then
	export NOWINDOW_INDEX
	NOWINDOW_INDEX="$(
		awk '
			/name: "nowindow"/ { hit = 1 }
			hit && /window: [0-9]+/ {
				sub(/.*window:[[:space:]]*/, "")
				sub(/[^0-9].*/, "")
				print
				exit
			}
		' "${_window_vocab_loader}"
	)"
fi
unset _window_vocab_root _window_vocab_authority _window_vocab_loader
