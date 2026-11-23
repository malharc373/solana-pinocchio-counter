//! LiteSVM tests. Run `cargo build-sbf` first (they load target/deploy/*.so).
//!
//! The same scenarios as the Anchor version, plus the checks Anchor used to do
//! for us and now have to be proven by hand: owner, discriminator, signer,
//! and the pre-funded-PDA case in `initialize`.

use {
    litesvm::LiteSVM,
    solana_instruction::{AccountMeta, Instruction},
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_pubkey::Pubkey,
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

const SOL: u64 = 1_000_000_000;
const SYSTEM: Pubkey = Pubkey::new_from_array([0; 32]);
const UNAUTHORIZED: &str = "custom program error: 0x0";
const UNDERFLOW: &str = "custom program error: 0x2";

fn program_id() -> Pubkey {
    Pubkey::new_from_array(pinocchio_counter::ID.to_bytes())
}

#[allow(deprecated)] // solana-rent 3.x field names; values are the SIMD-0194 ones
fn setup() -> LiteSVM {
    let mut svm = LiteSVM::new();
    // Devnet and mainnet run the SIMD-0194 rent layout (lamports_per_byte = 5080,
    // threshold = 1.0, read on 2026-11-23). LiteSVM 0.10 still seeds the old one
    // (3480 per byte-year, threshold 2.0). Pinocchio 0.12's `Rent` reads only the
    // first field as lamports-per-byte, so under the old layout it asks for half
    // the rent and `create_account` fails with InsufficientFundsForRent.
    svm.set_sysvar(&solana_rent::Rent {
        lamports_per_byte_year: 5080,
        exemption_threshold: 1.0,
        burn_percent: 50,
    });
    let so = include_bytes!("../target/deploy/pinocchio_counter.so");
    svm.add_program(program_id(), so).unwrap();
    svm
}

fn user(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), SOL).unwrap();
    kp
}

fn pda(authority: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"counter", authority.as_ref()], &program_id()).0
}

/// Sends one instruction; Ok(compute units) or Err(logs).
fn send(svm: &mut LiteSVM, ix: Instruction, signer: &Keypair) -> Result<u64, String> {
    let msg = Message::new_with_blockhash(&[ix], Some(&signer.pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[signer]).unwrap();
    let res = svm
        .send_transaction(tx)
        .map(|m| m.compute_units_consumed)
        .map_err(|e| format!("{:?}\n{}", e.err, e.meta.logs.join("\n")));
    svm.expire_blockhash();
    res
}

fn ix_initialize(authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        program_id(),
        &[0],
        vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(pda(authority), false),
            AccountMeta::new_readonly(SYSTEM, false),
        ],
    )
}

fn ix_update(tag: u8, counter: Pubkey, authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        program_id(),
        &[tag],
        vec![
            AccountMeta::new(counter, false),
            AccountMeta::new_readonly(*authority, true),
        ],
    )
}

fn ix_close(authority: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        program_id(),
        &[3],
        vec![
            AccountMeta::new(pda(authority), false),
            AccountMeta::new(*authority, true),
        ],
    )
}

/// Returns (authority, count, bump) read from the raw 42-byte layout.
fn read(svm: &LiteSVM, authority: &Pubkey) -> (Pubkey, u64, u8) {
    let d = svm.get_account(&pda(authority)).unwrap().data;
    assert_eq!(d.len(), 42);
    assert_eq!(d[0], 1, "discriminator");
    (
        Pubkey::try_from(&d[1..33]).unwrap(),
        u64::from_le_bytes(d[33..41].try_into().unwrap()),
        d[41],
    )
}

#[test]
fn initialize_increment_decrement() {
    let mut svm = setup();
    let alice = user(&mut svm);
    send(&mut svm, ix_initialize(&alice.pubkey()), &alice).unwrap();

    let (authority, count, bump) = read(&svm, &alice.pubkey());
    assert_eq!((authority, count), (alice.pubkey(), 0));
    let canonical =
        Pubkey::find_program_address(&[b"counter", alice.pubkey().as_ref()], &program_id()).1;
    assert_eq!(bump, canonical, "stores the canonical bump");

    send(
        &mut svm,
        ix_update(1, pda(&alice.pubkey()), &alice.pubkey()),
        &alice,
    )
    .unwrap();
    send(
        &mut svm,
        ix_update(1, pda(&alice.pubkey()), &alice.pubkey()),
        &alice,
    )
    .unwrap();
    assert_eq!(read(&svm, &alice.pubkey()).1, 2);
    send(
        &mut svm,
        ix_update(2, pda(&alice.pubkey()), &alice.pubkey()),
        &alice,
    )
    .unwrap();
    assert_eq!(read(&svm, &alice.pubkey()).1, 1);
}

#[test]
fn each_user_has_their_own_counter() {
    let mut svm = setup();
    let alice = user(&mut svm);
    let bob = user(&mut svm);
    send(&mut svm, ix_initialize(&alice.pubkey()), &alice).unwrap();
    send(&mut svm, ix_initialize(&bob.pubkey()), &bob).unwrap();
    send(
        &mut svm,
        ix_update(1, pda(&alice.pubkey()), &alice.pubkey()),
        &alice,
    )
    .unwrap();
    assert_eq!(read(&svm, &alice.pubkey()).1, 1);
    assert_eq!(read(&svm, &bob.pubkey()).1, 0);
}

#[test]
fn cannot_initialize_twice() {
    let mut svm = setup();
    let alice = user(&mut svm);
    send(&mut svm, ix_initialize(&alice.pubkey()), &alice).unwrap();
    send(
        &mut svm,
        ix_update(1, pda(&alice.pubkey()), &alice.pubkey()),
        &alice,
    )
    .unwrap();
    let logs = send(&mut svm, ix_initialize(&alice.pubkey()), &alice).unwrap_err();
    // Our own guard fires before any CPI (the System Program would also refuse).
    assert!(
        logs.contains("AccountAlreadyInitialized")
            && !logs.contains("11111111111111111111111111111111 invoke"),
        "{logs}"
    );
    assert_eq!(read(&svm, &alice.pubkey()).1, 1, "count was not reset");
}

#[test]
fn initialize_rejects_a_wrong_address() {
    let mut svm = setup();
    let alice = user(&mut svm);
    let mut ix = ix_initialize(&alice.pubkey());
    ix.accounts[1].pubkey = Pubkey::new_unique();
    assert!(send(&mut svm, ix, &alice).is_err());
}

#[test]
fn decrement_below_zero_fails() {
    let mut svm = setup();
    let alice = user(&mut svm);
    send(&mut svm, ix_initialize(&alice.pubkey()), &alice).unwrap();
    let logs = send(
        &mut svm,
        ix_update(2, pda(&alice.pubkey()), &alice.pubkey()),
        &alice,
    )
    .unwrap_err();
    assert!(logs.contains(UNDERFLOW), "{logs}");
}

#[test]
fn attacker_cannot_touch_someone_elses_counter() {
    let mut svm = setup();
    let alice = user(&mut svm);
    let mallory = user(&mut svm);
    send(&mut svm, ix_initialize(&alice.pubkey()), &alice).unwrap();

    for tag in [1, 2] {
        let logs = send(
            &mut svm,
            ix_update(tag, pda(&alice.pubkey()), &mallory.pubkey()),
            &mallory,
        )
        .unwrap_err();
        assert!(logs.contains(UNAUTHORIZED), "{logs}");
    }
    let close = Instruction::new_with_bytes(
        program_id(),
        &[3],
        vec![
            AccountMeta::new(pda(&alice.pubkey()), false),
            AccountMeta::new(mallory.pubkey(), true),
        ],
    );
    assert!(send(&mut svm, close, &mallory).is_err());
    assert_eq!(read(&svm, &alice.pubkey()).1, 0);
}

#[test]
fn authority_must_sign() {
    let mut svm = setup();
    let alice = user(&mut svm);
    let mallory = user(&mut svm);
    send(&mut svm, ix_initialize(&alice.pubkey()), &alice).unwrap();

    // Alice is listed as the authority but only Mallory signs (as fee payer).
    let mut ix = ix_update(1, pda(&alice.pubkey()), &alice.pubkey());
    ix.accounts[1].is_signer = false;
    let logs = send(&mut svm, ix, &mallory).unwrap_err();
    assert!(logs.contains("missing required signature"), "{logs}");
}

#[test]
fn rejects_a_forged_counter_owned_by_another_program() {
    let mut svm = setup();
    let mallory = user(&mut svm);

    // A byte-perfect counter for Mallory with count = 41, but owned by the
    // System Program instead of ours. Without the owner check it would pass.
    let mut data = vec![1u8];
    data.extend_from_slice(mallory.pubkey().as_ref());
    data.extend_from_slice(&41u64.to_le_bytes());
    data.push(
        Pubkey::find_program_address(&[b"counter", mallory.pubkey().as_ref()], &program_id()).1,
    );
    let fake = Pubkey::new_unique();
    svm.set_account(
        fake,
        solana_account::Account {
            lamports: SOL,
            data,
            owner: SYSTEM,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
    let logs = send(&mut svm, ix_update(1, fake, &mallory.pubkey()), &mallory).unwrap_err();
    assert!(logs.contains("incorrect program id"), "{logs}");
}

#[test]
fn rejects_unknown_instructions() {
    let mut svm = setup();
    let alice = user(&mut svm);
    for data in [vec![], vec![9]] {
        let ix = Instruction::new_with_bytes(program_id(), &data, vec![]);
        assert!(send(&mut svm, ix, &alice).is_err());
    }
}

#[test]
fn initialize_survives_a_prefunded_pda() {
    let mut svm = setup();
    let alice = user(&mut svm);
    let griefer = user(&mut svm);

    // Griefer sends 1 lamport to Alice's future PDA so `create_account` would fail.
    let transfer = Instruction::new_with_bytes(
        SYSTEM,
        &[2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
        vec![
            AccountMeta::new(griefer.pubkey(), true),
            AccountMeta::new(pda(&alice.pubkey()), false),
        ],
    );
    send(&mut svm, transfer, &griefer).unwrap();
    assert_eq!(svm.get_balance(&pda(&alice.pubkey())).unwrap(), 1);

    send(&mut svm, ix_initialize(&alice.pubkey()), &alice).unwrap();
    let acc = svm.get_account(&pda(&alice.pubkey())).unwrap();
    assert_eq!(acc.owner, program_id());
    assert_eq!(acc.lamports, svm.minimum_balance_for_rent_exemption(42));
    assert_eq!(read(&svm, &alice.pubkey()).1, 0);
}

#[test]
fn close_refunds_rent_and_allows_a_fresh_start() {
    let mut svm = setup();
    let alice = user(&mut svm);
    send(&mut svm, ix_initialize(&alice.pubkey()), &alice).unwrap();
    send(
        &mut svm,
        ix_update(1, pda(&alice.pubkey()), &alice.pubkey()),
        &alice,
    )
    .unwrap();

    let rent = svm.get_account(&pda(&alice.pubkey())).unwrap().lamports;
    let before = svm.get_balance(&alice.pubkey()).unwrap();
    send(&mut svm, ix_close(&alice.pubkey()), &alice).unwrap();
    assert_eq!(
        svm.get_balance(&alice.pubkey()).unwrap(),
        before + rent - 5000
    );
    assert!(svm
        .get_account(&pda(&alice.pubkey()))
        .is_none_or(|a| a.lamports == 0));

    // Re-creating starts from zero.
    send(&mut svm, ix_initialize(&alice.pubkey()), &alice).unwrap();
    assert_eq!(read(&svm, &alice.pubkey()).1, 0);
}
