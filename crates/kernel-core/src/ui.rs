//! Pure display UI state and touch hit-testing.
//!
//! The UI is deliberately a view model: it owns page selection and geometry,
//! never authority, device state, or configuration. The board renderer and
//! touch sampler consume this contract without adding policy to the kernel.

/// Physical panel width in landscape orientation.
pub const SCREEN_WIDTH: u16 = 480;
/// Physical panel height in landscape orientation.
pub const SCREEN_HEIGHT: u16 = 320;
/// Number of navigation tabs.
pub const TAB_COUNT: usize = 5;

/// Top-level dashboard pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Page {
    Overview = 0,
    Agents = 1,
    Resources = 2,
    Network = 3,
    Fault = 4,
}

impl Page {
    /// All pages in their stable navigation order.
    pub const ALL: [Self; TAB_COUNT] = [
        Self::Overview,
        Self::Agents,
        Self::Resources,
        Self::Network,
        Self::Fault,
    ];

    /// Numeric tab position.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Select a page by tab position, refusing out-of-range input.
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Overview),
            1 => Some(Self::Agents),
            2 => Some(Self::Resources),
            3 => Some(Self::Network),
            4 => Some(Self::Fault),
            _ => None,
        }
    }

    /// Next page in the automatic rotation order.
    pub const fn next(self) -> Self {
        match (self.index() + 1) % TAB_COUNT {
            0 => Self::Overview,
            1 => Self::Agents,
            2 => Self::Resources,
            3 => Self::Network,
            _ => Self::Fault,
        }
    }
}

/// Touch phase reported by a controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchKind {
    Down,
    Move,
    Up,
}

/// A normalized touch event in panel coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchEvent {
    pub kind: TouchKind,
    pub x: u16,
    pub y: u16,
}

/// Inclusive/exclusive rectangle used by hit-testing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    /// Whether a point is inside this rectangle.
    pub const fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

/// Fixed UI state, with no heap or device references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiState {
    page: Page,
    last_input_tick: u64,
    auto_rotate: bool,
}

impl UiState {
    /// Start on the overview page with automatic rotation enabled.
    pub const fn new() -> Self {
        Self {
            page: Page::Overview,
            last_input_tick: 0,
            auto_rotate: true,
        }
    }

    pub const fn page(self) -> Page {
        self.page
    }

    pub const fn auto_rotate(self) -> bool {
        self.auto_rotate
    }

    /// Change page only through a valid tab or the automatic timer.
    pub fn on_touch(&mut self, event: TouchEvent, now: u64) -> bool {
        if event.kind != TouchKind::Down || event.y < SCREEN_HEIGHT - 48 {
            return false;
        }
        let tab = usize::from(event.x) / (usize::from(SCREEN_WIDTH) / TAB_COUNT);
        let Some(page) = Page::from_index(tab) else {
            return false;
        };
        let changed = self.page != page;
        self.page = page;
        self.last_input_tick = now;
        self.auto_rotate = true;
        changed
    }

    /// Rotate after five seconds of inactivity (10 Hz timer => 50 ticks).
    pub fn on_tick(&mut self, now: u64) -> bool {
        if !self.auto_rotate || now.saturating_sub(self.last_input_tick) < 50 {
            return false;
        }
        self.last_input_tick = now;
        self.page = self.page.next();
        true
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_tabs_select_pages_and_reject_outside_points() {
        let mut ui = UiState::new();
        assert!(ui.on_touch(
            TouchEvent {
                kind: TouchKind::Down,
                x: 210,
                y: 300
            },
            3
        ));
        assert_eq!(ui.page(), Page::Resources);
        assert!(!ui.on_touch(
            TouchEvent {
                kind: TouchKind::Down,
                x: 20,
                y: 100
            },
            4
        ));
        assert_eq!(ui.page(), Page::Resources);
    }

    #[test]
    fn movement_and_release_do_not_change_navigation() {
        let mut ui = UiState::new();
        let event = TouchEvent {
            kind: TouchKind::Move,
            x: 400,
            y: 300,
        };
        assert!(!ui.on_touch(event, 0));
        assert_eq!(ui.page(), Page::Overview);
    }

    #[test]
    fn automatic_rotation_is_bounded_and_resets_after_touch() {
        let mut ui = UiState::new();
        assert!(!ui.on_tick(49));
        assert!(ui.on_tick(50));
        assert_eq!(ui.page(), Page::Agents);
        assert!(!ui.on_tick(51));
        assert!(ui.on_touch(
            TouchEvent {
                kind: TouchKind::Down,
                x: 10,
                y: 300
            },
            100
        ));
        assert!(!ui.on_tick(149));
        assert!(ui.on_tick(150));
    }

    #[test]
    fn rectangle_edges_are_half_open() {
        let r = Rect {
            x: 10,
            y: 20,
            width: 30,
            height: 40,
        };
        assert!(r.contains(10, 20));
        assert!(r.contains(39, 59));
        assert!(!r.contains(40, 59));
        assert!(!r.contains(39, 60));
    }
}
