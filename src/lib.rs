//! Per-user counter, written with Pinocchio instead of Anchor.
//!
//! Same behaviour as the Anchor version (github.com/malharc373/solana-counter):
//! every wallet gets one counter PDA at `["counter", authority]` that only it
//! can increment, decrement or close.
//!
//! What Anchor did for us and this program now does by hand:
//! - instruction dispatch (a 1-byte tag instead of an 8-byte discriminator)
//! - account checks: signer, owner, size, discriminator, PDA, stored authority
//! - zero-copy state: fields are read straight out of the account bytes
//! - account creation via a System Program CPI, including the case where
//!   someone pre-funded the PDA to block `create_account`
//! - closing: moving the lamports out and shrinking the account to zero

#![no_std]

pub mod error;
pub mod instructions;
pub mod state;

use pinocchio::{error::ProgramError, AccountView, Address, ProgramResult};

solana_address::declare_id!("DL4b35FZzH8ZnA8Nx6D67FxrnMrjYL5YHymhm3LWnFG1");

pub const COUNTER_SEED: &[u8] = b"counter";

/// Instruction tags (first byte of instruction data).
#[repr(u8)]
pub enum Ix {
    Initialize = 0,
    Increment = 1,
    Decrement = 2,
    Close = 3,
}

#[cfg(not(feature = "no-entrypoint"))]
mod entrypoint {
    use super::*;
    // At most 3 accounts per instruction: keeps the entrypoint's stack small.
    pinocchio::program_entrypoint!(process_instruction, 3);
    pinocchio::no_allocator!();
    pinocchio::nostd_panic_handler!();
}

pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    match data.first() {
        Some(&t) if t == Ix::Initialize as u8 => instructions::initialize(program_id, accounts),
        Some(&t) if t == Ix::Increment as u8 => instructions::update(program_id, accounts, 1),
        Some(&t) if t == Ix::Decrement as u8 => instructions::update(program_id, accounts, -1),
        Some(&t) if t == Ix::Close as u8 => instructions::close(program_id, accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
