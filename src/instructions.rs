use pinocchio::{
    cpi::{invoke, invoke_signed, Seed, Signer},
    error::ProgramError,
    instruction::{InstructionAccount, InstructionView},
    sysvars::{rent::Rent, Sysvar},
    AccountView, Address, ProgramResult,
};

use crate::{
    error::CounterError,
    state::{Counter, COUNTER_DISCRIMINATOR},
    COUNTER_SEED,
};

/// System Program ID (all zeroes).
const SYSTEM_PROGRAM_ID: Address = Address::new_from_array([0; 32]);

/// Accounts: `[authority (writable, signer), counter (writable), system_program]`.
///
/// Creates the caller's counter PDA, starting at 0.
pub fn initialize(program_id: &Address, accounts: &mut [AccountView]) -> ProgramResult {
    let [authority, counter, system_program, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if system_program.address() != &SYSTEM_PROGRAM_ID {
        return Err(ProgramError::IncorrectProgramId);
    }

    // Find the canonical bump (what Anchor's `init` + `bump` does).
    let (expected, bump) = find_pda(authority.address(), program_id)?;
    if counter.address() != &expected {
        return Err(ProgramError::InvalidSeeds);
    }
    // Refuse to re-initialize: an existing counter is owned by this program.
    if counter.owned_by(program_id) {
        return Err(ProgramError::AccountAlreadyInitialized);
    }

    let bump_seed = [bump];
    let seeds = [
        Seed::from(COUNTER_SEED),
        Seed::from(authority.address().as_ref()),
        Seed::from(&bump_seed),
    ];
    let signer = [Signer::from(&seeds)];
    create_pda_account(authority, counter, Counter::LEN, program_id, &signer)?;

    let mut data = counter.try_borrow_mut()?;
    let state = Counter::load_mut(&mut data)?;
    state.discriminator = COUNTER_DISCRIMINATOR;
    state.authority = *authority.address();
    state.set_count(0);
    state.bump = bump;
    Ok(())
}

/// Accounts: `[counter (writable), authority (signer)]`.
///
/// Adds `delta` (+1 or -1) with checked arithmetic.
pub fn update(program_id: &Address, accounts: &mut [AccountView], delta: i8) -> ProgramResult {
    let [counter, authority, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let address = *counter.address();
    check_owner(program_id, counter)?;
    let mut data = counter.try_borrow_mut()?;
    let state = load_checked(program_id, &address, &mut data, authority)?;
    let count = state.count();
    let next = if delta > 0 {
        count.checked_add(1).ok_or(CounterError::Overflow)?
    } else {
        count.checked_sub(1).ok_or(CounterError::Underflow)?
    };
    state.set_count(next);
    Ok(())
}

/// Accounts: `[counter (writable), authority (writable, signer)]`.
///
/// Deletes the counter and sends its rent back to the authority.
pub fn close(program_id: &Address, accounts: &mut [AccountView]) -> ProgramResult {
    let [counter, authority, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !authority.is_writable() {
        return Err(ProgramError::InvalidAccountData);
    }
    let address = *counter.address();
    check_owner(program_id, counter)?;
    {
        let mut data = counter.try_borrow_mut()?;
        let state = load_checked(program_id, &address, &mut data, authority)?;
        // Wipe the discriminator so the bytes can never be read as a Counter
        // again within this transaction.
        state.discriminator = 0;
    }
    let lamports = counter.lamports();
    authority.set_lamports(
        authority
            .lamports()
            .checked_add(lamports)
            .ok_or(ProgramError::ArithmeticOverflow)?,
    );
    counter.set_lamports(0);
    counter.close()
}

/// The rest of what Anchor's `Account<Counter>` + `seeds`/`bump` + `has_one`
/// checked. Call `check_owner` first.
fn load_checked<'a>(
    program_id: &Address,
    counter_address: &Address,
    data: &'a mut [u8],
    authority: &AccountView,
) -> Result<&'a mut Counter, ProgramError> {
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let state = Counter::load_mut(data)?;
    if state.discriminator != COUNTER_DISCRIMINATOR {
        return Err(ProgramError::InvalidAccountData);
    }
    if &state.authority != authority.address() {
        return Err(CounterError::Unauthorized.into());
    }
    // Re-derive the PDA by searching for the bump instead of using the stored one.
    let (expected, _) = find_pda(authority.address(), program_id)?;
    if counter_address != &expected {
        return Err(ProgramError::InvalidSeeds);
    }
    Ok(state)
}

/// Only accounts this program owns can be counters (Anchor's `Account<T>`
/// owner check). Done before borrowing so `load_checked` can take the bytes.
#[inline(always)]
fn check_owner(program_id: &Address, counter: &AccountView) -> ProgramResult {
    if !counter.owned_by(program_id) {
        return Err(ProgramError::IncorrectProgramId);
    }
    Ok(())
}

#[inline(always)]
fn find_pda(authority: &Address, program_id: &Address) -> Result<(Address, u8), ProgramError> {
    #[cfg(target_os = "solana")]
    {
        Address::try_find_program_address(&[COUNTER_SEED, authority.as_ref()], program_id)
            .ok_or(ProgramError::InvalidSeeds)
    }
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (authority, program_id);
        Err(ProgramError::InvalidSeeds)
    }
}

/// Creates `account` at a PDA, owned by `owner`, funded by `payer`.
///
/// `create_account` fails if the address already holds lamports, so anyone
/// could block a user's counter by sending 1 lamport to the PDA first. In that
/// case top up to rent-exempt, then `allocate` + `assign` (what Anchor does).
fn create_pda_account(
    payer: &AccountView,
    account: &AccountView,
    space: usize,
    owner: &Address,
    signer: &[Signer],
) -> ProgramResult {
    let rent = Rent::get()?.try_minimum_balance(space)?;
    let current = account.lamports();

    if current == 0 {
        // CreateAccount { lamports, space, owner }: tag 0 (u32 LE)
        let mut data = [0u8; 52];
        data[4..12].copy_from_slice(&rent.to_le_bytes());
        data[12..20].copy_from_slice(&(space as u64).to_le_bytes());
        data[20..52].copy_from_slice(owner.as_ref());
        let metas = [
            InstructionAccount::writable_signer(payer.address()),
            InstructionAccount::writable_signer(account.address()),
        ];
        let ix = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &metas,
        };
        return invoke_signed(&ix, &[payer, account], signer);
    }

    if current < rent {
        // Transfer { lamports }: tag 2
        let mut data = [0u8; 12];
        data[0] = 2;
        data[4..12].copy_from_slice(&(rent - current).to_le_bytes());
        let metas = [
            InstructionAccount::writable_signer(payer.address()),
            InstructionAccount::writable(account.address()),
        ];
        let ix = InstructionView {
            program_id: &SYSTEM_PROGRAM_ID,
            data: &data,
            accounts: &metas,
        };
        invoke(&ix, &[payer, account])?;
    }

    // Allocate { space }: tag 8
    let mut data = [0u8; 12];
    data[0] = 8;
    data[4..12].copy_from_slice(&(space as u64).to_le_bytes());
    let metas = [InstructionAccount::writable_signer(account.address())];
    let ix = InstructionView {
        program_id: &SYSTEM_PROGRAM_ID,
        data: &data,
        accounts: &metas,
    };
    invoke_signed(&ix, &[account], signer)?;

    // Assign { owner }: tag 1
    let mut data = [0u8; 36];
    data[0] = 1;
    data[4..36].copy_from_slice(owner.as_ref());
    let ix = InstructionView {
        program_id: &SYSTEM_PROGRAM_ID,
        data: &data,
        accounts: &metas,
    };
    invoke_signed(&ix, &[account], signer)
}
