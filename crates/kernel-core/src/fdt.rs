//! Flattened device-tree reader — the closed extraction list of ADR-0073.
//!
//! Pure and total over `&[u8]`: zero alloc, no recursion (explicit depth
//! counter), every read bounds-checked, big-endian decoded via
//! `u32::from_be_bytes`. The header is re-validated here even though
//! `arch::bootinfo::survey` checked the magic once — pure functions do not
//! trust callers.
//!
//! What it extracts (and nothing else — growth needs an ADR, ADR-0072 §3):
//! root `model` / first `compatible` / `#address-cells` / `#size-cells`,
//! `/memory*` `reg` ranges, the `/cpus` cpu count, and `/system`
//! `linux,revision` (patched in by the Pi firmware; absent from distributed
//! blobs). No phandles, no overlays, no memory-reservation block, no
//! `/chosen`, no `/soc`: board truth stays compiled (ADR-0011).

/// `FDT_MAGIC`, big endian on the wire.
pub const FDT_MAGIC: u32 = 0xd00d_feed;

const FDT_BEGIN_NODE: u32 = 0x1;
const FDT_END_NODE: u32 = 0x2;
const FDT_PROP: u32 = 0x3;
const FDT_NOP: u32 = 0x4;
const FDT_END: u32 = 0x9;

/// Deep enough for `/cpus/cpu@N/…`; a deeper tree is not one we read from.
const MAX_DEPTH: u32 = 8;

/// Ranges kept from `/memory` nodes; more are counted but not stored.
pub const MAX_MEM_RANGES: usize = 4;

/// Why a blob was refused. Every variant is a refusal of the whole parse:
/// a tree that lies about its own structure is not selectively trustworthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdtError {
    /// Shorter than its header, or a field points past the end.
    Truncated,
    /// First word is not `FDT_MAGIC`.
    BadMagic,
    /// Header version older than 17 (no `size_dt_struct`).
    OldVersion,
    /// A token, name, or property overruns its block.
    BadStructure,
    /// Node nesting beyond [`MAX_DEPTH`].
    TooDeep,
    /// `#address-cells` / `#size-cells` outside 1..=2.
    BadCells,
}

/// Inline NUL-free string, truncated visibly at capacity.
#[derive(Debug, Clone, Copy)]
pub struct Str64 {
    buf: [u8; 64],
    len: u8,
    truncated: bool,
}

impl Str64 {
    pub const fn empty() -> Self {
        Self {
            buf: [0; 64],
            len: 0,
            truncated: false,
        }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let mut buf = [0u8; 64];
        let take = bytes.len().min(64);
        buf[..take].copy_from_slice(&bytes[..take]);
        Self {
            buf,
            len: take as u8,
            truncated: bytes.len() > 64,
        }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("<non-utf8>")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// True when the source was longer than the 64-byte capacity.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// One `/memory` `reg` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemRange {
    pub base: u64,
    pub len: u64,
}

/// Everything the closed list extracts. Fields default to "absent", so a
/// tree that lacks a node yields zeros/None rather than an error — absence
/// is a fact to report, not a malformation.
#[derive(Debug, Clone, Copy)]
pub struct Extract {
    pub model: Str64,
    pub compatible: Str64,
    pub address_cells: u32,
    pub size_cells: u32,
    pub memory: [MemRange; MAX_MEM_RANGES],
    /// Ranges seen (may exceed the stored [`MAX_MEM_RANGES`]).
    pub memory_ranges: usize,
    /// Sum of every `/memory` range length, including unstored ones.
    pub memory_total: u64,
    pub cpus: u32,
    pub revision: Option<u32>,
}

impl Extract {
    const fn new() -> Self {
        Self {
            model: Str64::empty(),
            compatible: Str64::empty(),
            // Spec defaults, overridden by the root props when present.
            address_cells: 2,
            size_cells: 1,
            memory: [MemRange { base: 0, len: 0 }; MAX_MEM_RANGES],
            memory_ranges: 0,
            memory_total: 0,
            cpus: 0,
            revision: None,
        }
    }
}

fn be32(data: &[u8], off: usize) -> Result<u32, FdtError> {
    let bytes = data.get(off..off + 4).ok_or(FdtError::Truncated)?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// NUL-terminated string at `off`, bounded by `end`.
fn cstr(data: &[u8], off: usize, end: usize) -> Result<&[u8], FdtError> {
    let slice = data.get(off..end).ok_or(FdtError::BadStructure)?;
    let nul = slice
        .iter()
        .position(|&b| b == 0)
        .ok_or(FdtError::BadStructure)?;
    Ok(&slice[..nul])
}

/// Which top-level (or `/cpus`-child) context the walker is inside.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Node {
    Other,
    Memory,
    Cpus,
    System,
    /// Depth-3 child of `/cpus`; `true` once `device_type = "cpu"` is seen.
    CpusChild(bool),
}

/// Parse `data` and run the closed extraction list.
pub fn extract(data: &[u8]) -> Result<Extract, FdtError> {
    // Header: magic(0) totalsize(4) off_dt_struct(8) off_dt_strings(12)
    // off_mem_rsvmap(16) version(20) last_comp(24) boot_cpuid(28)
    // size_dt_strings(32) size_dt_struct(36).
    if data.len() < 40 {
        return Err(FdtError::Truncated);
    }
    if be32(data, 0)? != FDT_MAGIC {
        return Err(FdtError::BadMagic);
    }
    let total = be32(data, 4)? as usize;
    if total < 40 || total > data.len() {
        return Err(FdtError::Truncated);
    }
    if be32(data, 20)? < 17 {
        return Err(FdtError::OldVersion);
    }
    let struct_off = be32(data, 8)? as usize;
    let strings_off = be32(data, 12)? as usize;
    let strings_len = be32(data, 32)? as usize;
    let struct_len = be32(data, 36)? as usize;
    let struct_end = struct_off
        .checked_add(struct_len)
        .ok_or(FdtError::Truncated)?;
    let strings_end = strings_off
        .checked_add(strings_len)
        .ok_or(FdtError::Truncated)?;
    if struct_end > total || strings_end > total {
        return Err(FdtError::Truncated);
    }

    let mut out = Extract::new();
    let mut off = struct_off;
    let mut depth: u32 = 0;
    // Context of the nearest enclosing named node per depth of interest.
    let mut context = Node::Other;
    let mut token_budget = struct_len / 4 + 1;

    loop {
        // Every token is at least one word; a walk that outlives the block's
        // own word count is cyclic or corrupt.
        token_budget = token_budget.checked_sub(1).ok_or(FdtError::BadStructure)?;
        if off + 4 > struct_end {
            return Err(FdtError::BadStructure);
        }
        let token = be32(data, off)?;
        off += 4;
        match token {
            FDT_BEGIN_NODE => {
                let name = cstr(data, off, struct_end)?;
                off = (off + name.len() + 1).next_multiple_of(4);
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(FdtError::TooDeep);
                }
                // Depth 1 is the root; depth 2 its children; depth 3 the
                // `/cpus` children we count.
                if depth == 2 {
                    let bare = name.split(|&b| b == b'@').next().unwrap_or(name);
                    context = match bare {
                        b"memory" => Node::Memory,
                        b"cpus" => Node::Cpus,
                        b"system" => Node::System,
                        _ => Node::Other,
                    };
                } else if depth == 3 && context == Node::Cpus {
                    context = Node::CpusChild(false);
                }
            }
            FDT_END_NODE => {
                depth = depth.checked_sub(1).ok_or(FdtError::BadStructure)?;
                match context {
                    Node::CpusChild(counted) => {
                        if counted {
                            out.cpus += 1;
                        }
                        context = Node::Cpus;
                    }
                    _ if depth <= 1 => context = Node::Other,
                    _ => {}
                }
            }
            FDT_PROP => {
                let len = be32(data, off)? as usize;
                let name_off = be32(data, off + 4)? as usize;
                let value_off = off + 8;
                let value_end = value_off.checked_add(len).ok_or(FdtError::BadStructure)?;
                if value_end > struct_end {
                    return Err(FdtError::BadStructure);
                }
                let strings_at = strings_off
                    .checked_add(name_off)
                    .ok_or(FdtError::BadStructure)?;
                let name = cstr(data, strings_at, strings_end)?;
                let value = &data[value_off..value_end];
                absorb(&mut out, depth, &mut context, name, value)?;
                off = value_end.next_multiple_of(4);
            }
            FDT_NOP => {}
            FDT_END => {
                if depth != 0 {
                    return Err(FdtError::BadStructure);
                }
                return Ok(out);
            }
            _ => return Err(FdtError::BadStructure),
        }
    }
}

/// Fold one property into the extract, per the closed list.
fn absorb(
    out: &mut Extract,
    depth: u32,
    context: &mut Node,
    name: &[u8],
    value: &[u8],
) -> Result<(), FdtError> {
    if depth == 1 {
        match name {
            b"model" => out.model = Str64::from_bytes(strip_nul(value)),
            // Only the first (most specific) compatible entry.
            b"compatible" => {
                let first = value.split(|&b| b == 0).next().unwrap_or(&[]);
                out.compatible = Str64::from_bytes(first);
            }
            b"#address-cells" => out.address_cells = prop_u32(value)?,
            b"#size-cells" => out.size_cells = prop_u32(value)?,
            _ => {}
        }
        return Ok(());
    }
    match *context {
        Node::Memory if depth == 2 && name == b"reg" => {
            absorb_memory_reg(out, value)?;
        }
        Node::System if depth == 2 && name == b"linux,revision" => {
            out.revision = Some(prop_u32(value)?);
        }
        Node::CpusChild(_)
            if depth == 3 && name == b"device_type" && strip_nul(value) == b"cpu" =>
        {
            *context = Node::CpusChild(true);
        }
        _ => {}
    }
    Ok(())
}

fn strip_nul(value: &[u8]) -> &[u8] {
    value.strip_suffix(&[0]).unwrap_or(value)
}

fn prop_u32(value: &[u8]) -> Result<u32, FdtError> {
    if value.len() != 4 {
        return Err(FdtError::BadStructure);
    }
    Ok(u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
}

/// `reg` = N pairs of (`#address-cells` + `#size-cells`) cells.
fn absorb_memory_reg(out: &mut Extract, value: &[u8]) -> Result<(), FdtError> {
    let (ac, sc) = (out.address_cells as usize, out.size_cells as usize);
    if !(1..=2).contains(&ac) || !(1..=2).contains(&sc) {
        return Err(FdtError::BadCells);
    }
    let pair = (ac + sc) * 4;
    if pair == 0 || !value.len().is_multiple_of(pair) {
        return Err(FdtError::BadStructure);
    }
    let mut off = 0;
    while off < value.len() {
        let base = cells(value, off, ac)?;
        let len = cells(value, off + ac * 4, sc)?;
        if out.memory_ranges < MAX_MEM_RANGES {
            out.memory[out.memory_ranges] = MemRange { base, len };
        }
        out.memory_ranges += 1;
        out.memory_total = out.memory_total.saturating_add(len);
        off += pair;
    }
    Ok(())
}

fn cells(value: &[u8], off: usize, n: usize) -> Result<u64, FdtError> {
    let mut acc: u64 = 0;
    for i in 0..n {
        let word = value
            .get(off + i * 4..off + i * 4 + 4)
            .ok_or(FdtError::BadStructure)?;
        acc = (acc << 32) | u64::from(u32::from_be_bytes([word[0], word[1], word[2], word[3]]));
    }
    Ok(acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distributed (un-patched) blob: mixed cells, zero-size memory, no
    /// revision — the honest hard case the fixtures MANIFEST documents.
    const PI4: &[u8] = include_bytes!("../tests/fixtures/bcm2711-rpi-4-b.dtb");

    #[test]
    fn pi4_distributed_blob() {
        let x = extract(PI4).unwrap();
        assert_eq!(x.model.as_str(), "Raspberry Pi 4 Model B");
        assert_eq!(x.compatible.as_str(), "raspberrypi,4-model-b");
        assert_eq!(x.address_cells, 2);
        assert_eq!(x.size_cells, 1);
        assert_eq!(x.cpus, 4);
        // Firmware patches the real size at boot; the shipped blob says 0.
        assert_eq!(x.memory_ranges, 1);
        assert_eq!(x.memory_total, 0);
        assert_eq!(x.revision, None);
    }

    #[test]
    fn refuses_truncated_header() {
        assert!(matches!(extract(&PI4[..32]), Err(FdtError::Truncated)));
    }

    #[test]
    fn refuses_bad_magic() {
        let mut bad = PI4.to_vec();
        bad[0] ^= 0xff;
        assert!(matches!(extract(&bad), Err(FdtError::BadMagic)));
    }

    #[test]
    fn refuses_truncated_struct_block() {
        // Keep the header but cut the body: totalsize now overruns the slice.
        assert!(matches!(extract(&PI4[..2048]), Err(FdtError::Truncated)));
    }

    #[test]
    fn refuses_totalsize_beyond_slice() {
        let mut bad = PI4.to_vec();
        let huge = (bad.len() as u32 + 4).to_be_bytes();
        bad[4..8].copy_from_slice(&huge);
        assert!(matches!(extract(&bad), Err(FdtError::Truncated)));
    }

    #[test]
    fn refuses_depth_bomb() {
        // header + struct block of nested BEGIN_NODE "" tokens.
        let mut blob = header(4096, 40, 4096 - 40, 4000, 8);
        for _ in 0..16 {
            blob.extend_from_slice(&FDT_BEGIN_NODE.to_be_bytes());
            blob.extend_from_slice(&[0, 0, 0, 0]); // name "" + pad
        }
        blob.resize(4096, 0);
        assert!(matches!(extract(&blob), Err(FdtError::TooDeep)));
    }

    #[test]
    fn refuses_zero_cells_reg() {
        // Root with #address-cells=0 and a memory node with a reg prop.
        let mut strings = Vec::new();
        let s_addr = push_str(&mut strings, "#address-cells");
        let s_reg = push_str(&mut strings, "reg");
        let mut body = Vec::new();
        begin_node(&mut body, "");
        prop(&mut body, s_addr, &0u32.to_be_bytes());
        begin_node(&mut body, "memory@0");
        prop(&mut body, s_reg, &[0; 8]);
        end_node(&mut body);
        end_node(&mut body);
        body.extend_from_slice(&FDT_END.to_be_bytes());
        let blob = assemble(&body, &strings);
        assert!(matches!(extract(&blob), Err(FdtError::BadCells)));
    }

    #[test]
    fn old_version_refused() {
        let mut bad = PI4.to_vec();
        bad[20..24].copy_from_slice(&16u32.to_be_bytes());
        assert!(matches!(extract(&bad), Err(FdtError::OldVersion)));
    }

    // --- tiny builder for hostile fixtures ---

    fn header(total: u32, s_off: u32, s_len: u32, str_off: u32, str_len: u32) -> Vec<u8> {
        let mut h = Vec::new();
        for word in [
            FDT_MAGIC, total, s_off, str_off, 0, 17, 16, 0, str_len, s_len,
        ] {
            h.extend_from_slice(&word.to_be_bytes());
        }
        h
    }

    fn push_str(strings: &mut Vec<u8>, s: &str) -> u32 {
        let off = strings.len() as u32;
        strings.extend_from_slice(s.as_bytes());
        strings.push(0);
        off
    }

    fn begin_node(body: &mut Vec<u8>, name: &str) {
        body.extend_from_slice(&FDT_BEGIN_NODE.to_be_bytes());
        body.extend_from_slice(name.as_bytes());
        body.push(0);
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
    }

    fn end_node(body: &mut Vec<u8>) {
        body.extend_from_slice(&FDT_END_NODE.to_be_bytes());
    }

    fn prop(body: &mut Vec<u8>, name_off: u32, value: &[u8]) {
        body.extend_from_slice(&FDT_PROP.to_be_bytes());
        body.extend_from_slice(&(value.len() as u32).to_be_bytes());
        body.extend_from_slice(&name_off.to_be_bytes());
        body.extend_from_slice(value);
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
    }

    fn assemble(body: &[u8], strings: &[u8]) -> Vec<u8> {
        let s_off = 40u32;
        let str_off = s_off + body.len() as u32;
        let total = str_off + strings.len() as u32;
        let mut blob = header(
            total,
            s_off,
            body.len() as u32,
            str_off,
            strings.len() as u32,
        );
        blob.extend_from_slice(body);
        blob.extend_from_slice(strings);
        blob
    }
}
