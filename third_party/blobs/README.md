# Platform firmware blobs

Closed-source Raspberry Pi firmware stages required to reach our kernel.

| File | Role |
|------|------|
| `start4.elf` | VideoCore firmware: DRAM, clocks, loads `kernel8.img` |
| `fixup4.dat` | Companion fixup for `start4.elf` |

**Do not hand-edit these files.** Fetch a pinned revision:

```bash
make blobs
# or: ./scripts/fetch-blobs.sh
```

Provenance (tag + SHA-256) is recorded in `MANIFEST.txt` after fetch.

Policy and rationale: [`docs/blobs.md`](../../docs/blobs.md).
