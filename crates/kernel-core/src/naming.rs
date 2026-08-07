//! Name → CapId registry (ADR-0035 / P5) — pure, host-tested.
//!
//! Short fixed names map to capability ids. Creators bind; resolve does not
//! check whether the CapId still names a live endpoint (that is IPC's job).

use crate::cap::CapId;

/// Concurrent bindings.
pub const MAX_NAMES: usize = 8;

/// Max bytes in a name (not including a terminator).
pub const MAX_NAME_LEN: usize = 16;

/// Why [`Table::bind`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    /// Empty name or longer than [`MAX_NAME_LEN`].
    BadName,
    /// Table full and name was not already bound (replace would succeed).
    Full,
}

/// Why [`Table::resolve`] / [`Table::unbind`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// No binding for this name.
    Missing,
    /// Empty or too long.
    BadName,
}

#[derive(Clone, Copy)]
struct Entry {
    live: bool,
    name_len: u8,
    name: [u8; MAX_NAME_LEN],
    cap: CapId,
}

impl Entry {
    const EMPTY: Self = Self {
        live: false,
        name_len: 0,
        name: [0; MAX_NAME_LEN],
        cap: CapId::from_raw(0),
    };
}

/// Pure name registry.
#[derive(Clone)]
pub struct Table {
    entries: [Entry; MAX_NAMES],
}

impl Table {
    pub const fn new() -> Self {
        Self {
            entries: [Entry::EMPTY; MAX_NAMES],
        }
    }

    /// Bind `name` to `cap`. Replaces an existing binding of the same name.
    pub fn bind(&mut self, name: &[u8], cap: CapId) -> Result<(), BindError> {
        let len = validated_len(name).ok_or(BindError::BadName)?;
        if let Some(i) = self.find(name) {
            self.entries[i].cap = cap;
            self.entries[i].live = true;
            return Ok(());
        }
        for e in &mut self.entries {
            if !e.live {
                e.live = true;
                e.name_len = len as u8;
                e.name = [0; MAX_NAME_LEN];
                e.name[..len].copy_from_slice(&name[..len]);
                e.cap = cap;
                return Ok(());
            }
        }
        Err(BindError::Full)
    }

    /// Look up `name`.
    pub fn resolve(&self, name: &[u8]) -> Result<CapId, ResolveError> {
        let _ = validated_len(name).ok_or(ResolveError::BadName)?;
        let i = self.find(name).ok_or(ResolveError::Missing)?;
        Ok(self.entries[i].cap)
    }

    /// Remove a binding.
    pub fn unbind(&mut self, name: &[u8]) -> Result<(), ResolveError> {
        let _ = validated_len(name).ok_or(ResolveError::BadName)?;
        let i = self.find(name).ok_or(ResolveError::Missing)?;
        self.entries[i] = Entry::EMPTY;
        Ok(())
    }

    fn find(&self, name: &[u8]) -> Option<usize> {
        let len = name.len();
        for (i, e) in self.entries.iter().enumerate() {
            if e.live && e.name_len as usize == len && e.name[..len] == name[..] {
                return Some(i);
            }
        }
        None
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

fn validated_len(name: &[u8]) -> Option<usize> {
    let len = name.len();
    if len == 0 || len > MAX_NAME_LEN {
        None
    } else {
        Some(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_resolve_round_trip() {
        let mut t = Table::new();
        let cap = CapId::new(3, 7);
        t.bind(b"console", cap).unwrap();
        assert_eq!(t.resolve(b"console"), Ok(cap));
    }

    #[test]
    fn missing_name_is_refused() {
        let t = Table::new();
        assert_eq!(t.resolve(b"nope"), Err(ResolveError::Missing));
    }

    #[test]
    fn empty_and_long_names_are_bad() {
        let mut t = Table::new();
        let cap = CapId::new(1, 1);
        assert_eq!(t.bind(b"", cap), Err(BindError::BadName));
        assert_eq!(t.resolve(b""), Err(ResolveError::BadName));
        let long = [b'a'; MAX_NAME_LEN + 1];
        assert_eq!(t.bind(&long, cap), Err(BindError::BadName));
    }

    #[test]
    fn rebind_replaces_cap() {
        let mut t = Table::new();
        let a = CapId::new(1, 1);
        let b = CapId::new(2, 2);
        t.bind(b"svc", a).unwrap();
        t.bind(b"svc", b).unwrap();
        assert_eq!(t.resolve(b"svc"), Ok(b));
    }

    #[test]
    fn unbind_then_missing() {
        let mut t = Table::new();
        t.bind(b"x", CapId::new(1, 1)).unwrap();
        t.unbind(b"x").unwrap();
        assert_eq!(t.resolve(b"x"), Err(ResolveError::Missing));
        assert_eq!(t.unbind(b"x"), Err(ResolveError::Missing));
    }

    #[test]
    fn full_table_refuses_new_name() {
        let mut t = Table::new();
        for i in 0..MAX_NAMES {
            let name = [b'a' + i as u8];
            t.bind(&name, CapId::new(i as u16, 1)).unwrap();
        }
        assert_eq!(t.bind(b"z", CapId::new(9, 1)), Err(BindError::Full));
        // Rebind existing still works.
        t.bind(b"a", CapId::new(0, 2)).unwrap();
        assert_eq!(t.resolve(b"a"), Ok(CapId::new(0, 2)));
    }

    #[test]
    fn names_are_exact_match() {
        let mut t = Table::new();
        t.bind(b"ab", CapId::new(1, 1)).unwrap();
        assert_eq!(t.resolve(b"a"), Err(ResolveError::Missing));
        assert_eq!(t.resolve(b"abc"), Err(ResolveError::Missing));
    }
}
