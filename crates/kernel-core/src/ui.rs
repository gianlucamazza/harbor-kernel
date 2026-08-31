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
pub const TAB_COUNT: usize = 6;

/// Top-level dashboard pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Page {
    Overview = 0,
    Agents = 1,
    Resources = 2,
    Network = 3,
    Calibration = 4,
    Fault = 5,
}

impl Page {
    /// All pages in their stable navigation order.
    pub const ALL: [Self; TAB_COUNT] = [
        Self::Overview,
        Self::Agents,
        Self::Resources,
        Self::Network,
        Self::Calibration,
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
            4 => Some(Self::Calibration),
            5 => Some(Self::Fault),
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
            4 => Self::Calibration,
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

/// Operational state shown by the dashboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiMode {
    Boot,
    Ready,
    Degraded,
    Fault,
    Confirm,
}

/// Actions emitted by the pure UI model. Privileged actions are only proposed;
/// an owning service must validate the corresponding capability and confirm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiAction {
    None,
    Navigate(Page),
    RequestRecovery,
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
    mode: UiMode,
    last_input_tick: u64,
    auto_rotate: bool,
    pressed_at: Option<(u16, u16, u64)>,
}

impl UiState {
    /// Start on the overview page with automatic rotation enabled.
    pub const fn new() -> Self {
        Self {
            page: Page::Overview,
            mode: UiMode::Ready,
            last_input_tick: 0,
            auto_rotate: true,
            pressed_at: None,
        }
    }

    pub const fn page(self) -> Page {
        self.page
    }

    pub const fn auto_rotate(self) -> bool {
        self.auto_rotate
    }

    pub const fn mode(self) -> UiMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: UiMode) {
        self.mode = mode;
        if matches!(mode, UiMode::Fault | UiMode::Confirm) {
            self.auto_rotate = false;
        }
    }

    /// Handle one normalized event and emit only a bounded UI intent.
    pub fn handle_touch(&mut self, event: TouchEvent, now: u64) -> UiAction {
        match event.kind {
            TouchKind::Down => {
                self.pressed_at = Some((event.x, event.y, now));
                UiAction::None
            }
            TouchKind::Move => UiAction::None,
            TouchKind::Up => {
                let Some((x, y, started)) = self.pressed_at.take() else {
                    return UiAction::None;
                };
                let dx = x.abs_diff(event.x);
                let dy = y.abs_diff(event.y);
                if dx > 24 || dy > 24 || now.saturating_sub(started) > 20 {
                    return UiAction::None;
                }
                if event.y < SCREEN_HEIGHT - 48 {
                    return UiAction::None;
                }
                let tab = usize::from(event.x) / (usize::from(SCREEN_WIDTH) / TAB_COUNT);
                let Some(page) = Page::from_index(tab) else {
                    return UiAction::None;
                };
                self.last_input_tick = now;
                self.auto_rotate = !matches!(self.mode, UiMode::Fault | UiMode::Confirm);
                if self.page == page {
                    UiAction::None
                } else {
                    self.page = page;
                    UiAction::Navigate(page)
                }
            }
        }
    }

    /// Rotate after five seconds of inactivity (10 Hz timer => 50 ticks).
    pub fn on_tick(&mut self, now: u64) -> bool {
        if !self.auto_rotate
            || matches!(self.mode, UiMode::Boot | UiMode::Fault | UiMode::Confirm)
            || now.saturating_sub(self.last_input_tick) < 50
        {
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
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Down,
                    x: 210,
                    y: 300
                },
                3
            ),
            UiAction::None
        );
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Up,
                    x: 210,
                    y: 300
                },
                4
            ),
            UiAction::Navigate(Page::Resources)
        );
        assert_eq!(ui.page(), Page::Resources);
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Down,
                    x: 20,
                    y: 100
                },
                5
            ),
            UiAction::None
        );
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Up,
                    x: 20,
                    y: 100
                },
                6
            ),
            UiAction::None
        );
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
        assert_eq!(ui.handle_touch(event, 0), UiAction::None);
        assert_eq!(ui.page(), Page::Overview);
    }

    #[test]
    fn navigation_requires_a_short_stable_tap() {
        let mut ui = UiState::new();
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Down,
                    x: 210,
                    y: 300
                },
                1
            ),
            UiAction::None
        );
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Up,
                    x: 210,
                    y: 300
                },
                2
            ),
            UiAction::Navigate(Page::Resources)
        );
        assert_eq!(ui.page(), Page::Resources);
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Down,
                    x: 110,
                    y: 300
                },
                3
            ),
            UiAction::None
        );
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Up,
                    x: 140,
                    y: 300
                },
                4
            ),
            UiAction::None
        );
    }

    #[test]
    fn fault_mode_freezes_rotation_but_keeps_safe_navigation() {
        let mut ui = UiState::new();
        ui.set_mode(UiMode::Fault);
        assert!(!ui.on_tick(100));
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Down,
                    x: 110,
                    y: 300
                },
                101
            ),
            UiAction::None
        );
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Up,
                    x: 110,
                    y: 300
                },
                102
            ),
            UiAction::Navigate(Page::Agents)
        );
    }

    #[test]
    fn automatic_rotation_is_bounded_and_resets_after_touch() {
        let mut ui = UiState::new();
        assert!(!ui.on_tick(49));
        assert!(ui.on_tick(50));
        assert_eq!(ui.page(), Page::Agents);
        assert!(!ui.on_tick(51));
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Down,
                    x: 10,
                    y: 300
                },
                100
            ),
            UiAction::None
        );
        assert_eq!(
            ui.handle_touch(
                TouchEvent {
                    kind: TouchKind::Up,
                    x: 10,
                    y: 300
                },
                101
            ),
            UiAction::Navigate(Page::Overview)
        );
        assert!(!ui.on_tick(150));
        assert!(ui.on_tick(151));
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

    #[test]
    fn calibration_page_is_part_of_the_safe_navigation_order() {
        assert_eq!(TAB_COUNT, 6);
        assert_eq!(Page::from_index(4), Some(Page::Calibration));
        assert_eq!(Page::Calibration.next(), Page::Fault);
        assert_eq!(Page::Fault.next(), Page::Overview);
    }
}
