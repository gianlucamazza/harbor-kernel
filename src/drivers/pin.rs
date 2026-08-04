//! Digital output pin contract.
//!
//! Shape aligned with embedded-hal 1.0 `OutputPin`, implemented locally so
//! panel and bus helpers stay board-agnostic without pulling that crate into
//! the kernel (ADR-0009).

/// A pin driven as a push-pull output.
pub trait OutputPin {
    /// Failure mode of this pin implementation.
    type Error;

    /// Drive the pin high.
    fn set_high(&mut self) -> Result<(), Self::Error>;

    /// Drive the pin low.
    fn set_low(&mut self) -> Result<(), Self::Error>;
}

impl<T: OutputPin + ?Sized> OutputPin for &mut T {
    type Error = T::Error;

    #[inline]
    fn set_high(&mut self) -> Result<(), Self::Error> {
        (*self).set_high()
    }

    #[inline]
    fn set_low(&mut self) -> Result<(), Self::Error> {
        (*self).set_low()
    }
}
