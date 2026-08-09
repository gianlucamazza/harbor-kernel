#!/usr/bin/env bash
# A `DAIF` save/restore pair must not span a call that can switch tasks
# (ADR-0022 §2).
#
# `cpu::without_irqs(f)` reads `DAIF` before `f` and writes it back after. If a
# task switch happens in between, the saved value crosses into another task's
# execution: the next task runs with this task's mask, and this task later
# restores a value captured in an epoch that has ended.
#
# That is not a race to be closed by ordering — it is a scoping error, and until
# ADR-0022 the code avoided it only because nothing inside the scope ever
# switched. Blocking `SYS_RECV` changed that, so the rule needs a checker rather
# than a memory.
#
# ## Why this one is not a line regex
#
# Every other gate here greps lines. A scope is not a line: the offending call
# can be forty lines below the `without_irqs(` that opens the region, and the
# region can contain nested closures and braces inside strings. So this walks
# from each `without_irqs(` to its matching close, tracking depth, and searches
# what is actually inside.
#
# ## What counts as switching, and what deliberately does not
#
# The list is calls that **switch**, not calls into the scheduler:
#
#   - `block_current`, `yield_now`, `exit` — the three `switch_with` kinds
#   - `switch_with`, `context_switch`      — the machinery itself
#
# `sched::wake_task` is *not* on it, and that is the distinction the list
# encodes: it makes a task Ready and returns, and it takes `without_irqs`
# itself. Adding it would fail the scheduler's own source and teach the next
# person that this gate is noise.
#
# `sched::exit` is matched with its path, because a bare `exit` appears in
# unrelated contexts (`process::exit`, an `exit` field, prose).
#
# ## What it does not see, stated rather than implied
#
# The check is **lexical**. `ipc::recv_from_slot` switches — it parks — but it
# does so three frames down, so a call to it inside a masked region passes here.
# Catching that needs a call graph, which this tree does not have and which
# would be a large thing to build for one rule. What the gate buys is that the
# *direct* form, which is how the mistake is actually written, cannot land
# unnoticed. The indirect form remains review's job, and `docs/verification.md`
# lists it among the gate blind spots rather than leaving it to be discovered.
#
# Seen red: `sched::yield_now()` placed inside the session loop's masked step in
# `src/agent/mod.rs` — reported as `src/agent/mod.rs:178: \`yield_now\` is inside
# the \`without_irqs\` opened at line 177`, exit 1.
set -euo pipefail

cd "$(dirname "$0")/../.."

# The walker below keys on `without_irqs(` — a hand-rolled `cpu::irq_save()` /
# `irq_restore` pair is the same masked region without the name, and the walker
# never opens it (excellence review 2026-08-08, F-13: `taskcap::revoke_task`
# landed inside exactly such a region, benign but unwatched). The region cannot
# be walked the same way — `switch_with` legitimately contains the switch — so
# this refuses *new sites* instead. The two allowed files are the primitive's
# own implementation and the scheduler's switch path, each argued in place.
allowed_raw='src/arch/aarch64/cpu.rs src/sched/mod.rs'
raw_violations=0
while IFS= read -r hit; do
	[[ -z "${hit}" ]] && continue
	file="${hit%%:*}"
	rest="${hit#*:}"
	line="${rest%%:*}"
	content="${rest#*:}"
	content="${content%%//*}"
	[[ "${content}" == *"irq_save()"* ]] || continue
	case " ${allowed_raw} " in
	*" ${file} "*) ;;
	*)
		echo "irq-scope: ${file}:${line}: raw cpu::irq_save outside the audited set" >&2
		echo "  Use cpu::without_irqs so the scope walker can see inside the region," >&2
		echo "  or add the file to allowed_raw here with its argument." >&2
		raw_violations=$((raw_violations + 1))
		;;
	esac
done < <(grep -rn 'irq_save()' src crates --include='*.rs' || true)
if [[ "${raw_violations}" -ne 0 ]]; then
	exit 1
fi

python3 - <<'PY'
import io, re, subprocess, sys

OPENER = "without_irqs"
# Calls that can switch tasks. `wake_task` is deliberately absent — see above.
SWITCHERS = re.compile(
    r"\b(?:block_current|yield_now|switch_with|context_switch|sched::exit|preempt_switch)\s*\("
)

files = subprocess.run(
    ["bash", "-c", 'find src crates -name "*.rs"'],
    capture_output=True, text=True, check=True,
).stdout.split()


def strip_noise(text):
    """Blank out line comments and string literals, preserving offsets.

    Offsets have to survive: the report names a line number, and a rewrite that
    shortened the text would name the wrong one. Braces inside a string or a
    comment would otherwise unbalance the depth walk — `"{"` is one character
    that is not a scope.
    """
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        two = text[i:i + 2]
        if two == "//":
            while i < n and text[i] != "\n":
                out[i] = " "
                i += 1
        elif two == "/*":
            depth = 1
            out[i] = out[i + 1] = " "
            i += 2
            while i < n and depth:
                if text[i:i + 2] == "/*":
                    depth += 1
                    out[i] = out[i + 1] = " "
                    i += 2
                elif text[i:i + 2] == "*/":
                    depth -= 1
                    out[i] = out[i + 1] = " "
                    i += 2
                else:
                    if text[i] != "\n":
                        out[i] = " "
                    i += 1
        elif text[i] == '"':
            out[i] = " "
            i += 1
            while i < n and text[i] != '"':
                if text[i] == "\\":
                    out[i] = " "
                    i += 1
                    if i < n:
                        out[i] = " "
                        i += 1
                    continue
                if text[i] != "\n":
                    out[i] = " "
                i += 1
            if i < n:
                out[i] = " "
                i += 1
        else:
            i += 1
    return "".join(out)


violations = 0
regions = 0
for path in files:
    raw = io.open(path, encoding="utf-8").read()
    text = strip_noise(raw)
    for opener in re.finditer(r"\b" + OPENER + r"\s*\(", text):
        start = opener.end() - 1          # the '(' itself
        depth = 0
        end = None
        for j in range(start, len(text)):
            c = text[j]
            if c in "([{":
                depth += 1
            elif c in ")]}":
                depth -= 1
                if depth == 0:
                    end = j
                    break
        if end is None:
            # An unbalanced region means this file parsed wrong, and a checker
            # that shrugs at that is a checker that passes on the day it
            # matters. Say so and fail.
            print(f"irq-scope: {path}: unbalanced `{OPENER}(` at offset {start}", file=sys.stderr)
            violations += 1
            continue
        regions += 1
        body = text[start:end]
        for hit in SWITCHERS.finditer(body):
            open_line = text.count("\n", 0, start) + 1
            hit_line = text.count("\n", 0, start + hit.start()) + 1
            call = hit.group(0).rstrip("( \t")
            print(
                f"irq-scope: {path}:{hit_line}: `{call}` is inside the "
                f"`{OPENER}` opened at line {open_line}",
                file=sys.stderr,
            )
            violations += 1

if violations:
    print(f"irq-scope: {violations} switch(es) inside a masked region", file=sys.stderr)
    print("  `without_irqs` saves DAIF on entry and restores it on exit. A switch", file=sys.stderr)
    print("  between the two hands the next task this task's mask, and restores a", file=sys.stderr)
    print("  value captured in an epoch that has ended. Shrink the region to the", file=sys.stderr)
    print("  step that needs the mask, and switch outside it.", file=sys.stderr)
    print("  See docs/adr/0022-blocking-recv-and-the-mask-that-travels.md.", file=sys.stderr)
    sys.exit(1)

print(f"irq-scope: clean ({regions} masked regions, none containing a task switch)")
PY
