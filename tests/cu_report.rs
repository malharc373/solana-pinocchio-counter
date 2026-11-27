//! Deterministic compute-unit report for CI regression checks.
//!
//!   cargo build-sbf && cargo test --test cu_report
//!
//! Writes `target/cu-report.json` ({"instruction": compute units}) with the
//! `cu-report` crate. The cu-guard action compares it against the same report
//! built from the base branch.
//! The wallet comes from a fixed secret key, so the counter PDA and its bump are
//! the same on every run and so are the compute units. (`tests/bench.rs` uses
//! random wallets and averages; that suits a comparison, not a regression gate.)

use {
    litesvm::LiteSVM,
    solana_instruction::{AccountMeta, Instruction},
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_pubkey::Pubkey,
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

const SYSTEM: Pubkey = Pubkey::new_from_array([0; 32]);

#[test]
#[allow(deprecated)]
fn write_cu_report() {
    let id = Pubkey::new_from_array(pinocchio_counter::ID.to_bytes());
    let mut svm = LiteSVM::new();
    svm.set_sysvar(&solana_rent::Rent {
        lamports_per_byte_year: 5080,
        exemption_threshold: 1.0,
        burn_percent: 50,
    });
    svm.add_program(id, include_bytes!("../target/deploy/pinocchio_counter.so"))
        .unwrap();

    let user = Keypair::new_from_array([7; 32]);
    let u = user.pubkey();
    svm.airdrop(&u, 1_000_000_000).unwrap();
    let pda = Pubkey::find_program_address(&[b"counter", u.as_ref()], &id).0;

    let steps: [(&str, u8, Vec<AccountMeta>); 4] = [
        (
            "initialize",
            0,
            vec![
                AccountMeta::new(u, true),
                AccountMeta::new(pda, false),
                AccountMeta::new_readonly(SYSTEM, false),
            ],
        ),
        (
            "increment",
            1,
            vec![
                AccountMeta::new(pda, false),
                AccountMeta::new_readonly(u, true),
            ],
        ),
        (
            "decrement",
            2,
            vec![
                AccountMeta::new(pda, false),
                AccountMeta::new_readonly(u, true),
            ],
        ),
        (
            "close",
            3,
            vec![AccountMeta::new(pda, false), AccountMeta::new(u, true)],
        ),
    ];

    let mut report = cu_report::Report::new();
    for (name, tag, metas) in steps {
        let ix = Instruction::new_with_bytes(id, &[tag], metas);
        let msg = Message::new_with_blockhash(&[ix], Some(&u), &svm.latest_blockhash());
        let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[&user]).unwrap();
        let meta = svm
            .send_transaction(tx)
            .unwrap_or_else(|e| panic!("{name}: {:?}\n{}", e.err, e.meta.logs.join("\n")));
        svm.expire_blockhash();
        report.record(name, meta.compute_units_consumed);
    }
    let path = report.write(cu_report::default_path!()).unwrap();
    println!("{}:\n{}", path.display(), report.to_json());
}
