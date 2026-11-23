//! Account layout (42 bytes, no padding, read in place):
//!
//! | offset | size | field         |
//! |--------|------|---------------|
//! | 0      | 1    | discriminator |
//! | 1      | 32   | authority     |
//! | 33     | 8    | count (LE)    |
//! | 41     | 1    | bump          |

use pinocchio::{error::ProgramError, Address};

/// Marks the account as a `Counter` (Anchor uses 8 bytes; one is enough here).
pub const COUNTER_DISCRIMINATOR: u8 = 1;

#[repr(C)]
pub struct Counter {
    pub discriminator: u8,
    pub authority: Address,
    count: [u8; 8],
    pub bump: u8,
}

impl Counter {
    pub const LEN: usize = core::mem::size_of::<Self>();

    /// Views account bytes as a `Counter`. Every field is a byte array, so the
    /// struct has alignment 1 and any 42-byte slice is a valid `Counter`.
    pub fn load_mut(data: &mut [u8]) -> Result<&mut Self, ProgramError> {
        if data.len() != Self::LEN {
            return Err(ProgramError::InvalidAccountData);
        }
        // SAFETY: length checked above; `Counter` is `repr(C)` with align 1.
        Ok(unsafe { &mut *(data.as_mut_ptr() as *mut Self) })
    }

    pub fn count(&self) -> u64 {
        u64::from_le_bytes(self.count)
    }

    pub fn set_count(&mut self, value: u64) {
        self.count = value.to_le_bytes();
    }
}

const _: () = assert!(Counter::LEN == 42 && core::mem::align_of::<Counter>() == 1);
