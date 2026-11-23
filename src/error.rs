use pinocchio::error::ProgramError;

/// Same meaning as the Anchor version's errors.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterError {
    /// The signer is not the counter's authority.
    Unauthorized = 0,
    /// Counter would overflow.
    Overflow = 1,
    /// Counter is already at zero.
    Underflow = 2,
}

impl From<CounterError> for ProgramError {
    fn from(e: CounterError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
