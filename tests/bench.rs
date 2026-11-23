//! Compute-unit comparison: Anchor vs Pinocchio, same flow, same VM.
//!
//!   cargo build-sbf && cargo test --test bench -- --nocapture
//!
//! `bench/anchor_counter.so` is the Anchor 1.1.2 build of
//! github.com/malharc373/solana-counter at commit 70cb10e (deployed on devnet as
//! HoBHp5ZXWSwcVP7BH15K1qnn7Zn8mhgTZSEYUJLmwmeW). It logs on every instruction
//! ("Instruction: X" plus a `msg!`), and the Pinocchio version logs nothing, so
//! `bench/anchor_counter_nolog.so` is the same commit with every `msg!` removed
//! and built with `--features no-log-ix-name`: the fair comparison.

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
const ANCHOR_ID: Pubkey = Pubkey::from_str_const("HoBHp5ZXWSwcVP7BH15K1qnn7Zn8mhgTZSEYUJLmwmeW");

struct Program {
    name: &'static str,
    id: Pubkey,
    so: &'static [u8],
    /// initialize, increment, decrement, close
    data: [Vec<u8>; 4],
}

fn anchor_data() -> [Vec<u8>; 4] {
    [
        vec![175, 175, 109, 31, 13, 152, 155, 237],
        vec![11, 18, 104, 9, 104, 174, 59, 33],
        vec![106, 227, 168, 59, 248, 27, 150, 101],
        vec![98, 165, 201, 177, 108, 65, 206, 96],
    ]
}

fn programs() -> [Program; 3] {
    [
        Program {
            name: "Anchor 1.1.2",
            id: ANCHOR_ID,
            so: include_bytes!("../bench/anchor_counter.so"),
            data: anchor_data(),
        },
        Program {
            // Same declared ID: Anchor checks account owners against it.
            name: "Anchor, no logs",
            id: ANCHOR_ID,
            so: include_bytes!("../bench/anchor_counter_nolog.so"),
            data: anchor_data(),
        },
        Program {
            name: "Pinocchio 0.12",
            id: Pubkey::new_from_array(pinocchio_counter::ID.to_bytes()),
            so: include_bytes!("../target/deploy/pinocchio_counter.so"),
            data: [vec![0], vec![1], vec![2], vec![3]],
        },
    ]
}

#[allow(deprecated)]
fn setup(p: &Program) -> LiteSVM {
    let mut svm = LiteSVM::new();
    // Live devnet/mainnet rent (see tests/test_counter.rs).
    svm.set_sysvar(&solana_rent::Rent {
        lamports_per_byte_year: 5080,
        exemption_threshold: 1.0,
        burn_percent: 50,
    });
    svm.add_program(p.id, p.so).unwrap();
    svm
}

fn send(svm: &mut LiteSVM, ix: Instruction, signer: &Keypair) -> u64 {
    let msg = Message::new_with_blockhash(&[ix], Some(&signer.pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[signer]).unwrap();
    let meta = svm
        .send_transaction(tx)
        .unwrap_or_else(|e| panic!("{:?}\n{}", e.err, e.meta.logs.join("\n")));
    svm.expire_blockhash();
    meta.compute_units_consumed
}

/// Runs initialize → increment → decrement → close; returns CU per step.
fn run(svm: &mut LiteSVM, p: &Program) -> [u64; 4] {
    let user = Keypair::new();
    svm.airdrop(&user.pubkey(), 1_000_000_000).unwrap();
    let pda = Pubkey::find_program_address(&[b"counter", user.pubkey().as_ref()], &p.id).0;
    let u = user.pubkey();
    let accounts = [
        vec![
            AccountMeta::new(u, true),
            AccountMeta::new(pda, false),
            AccountMeta::new_readonly(SYSTEM, false),
        ],
        vec![
            AccountMeta::new(pda, false),
            AccountMeta::new_readonly(u, true),
        ],
        vec![
            AccountMeta::new(pda, false),
            AccountMeta::new_readonly(u, true),
        ],
        vec![AccountMeta::new(pda, false), AccountMeta::new(u, true)],
    ];
    let mut cu = [0; 4];
    for (i, metas) in accounts.into_iter().enumerate() {
        cu[i] = send(
            svm,
            Instruction::new_with_bytes(p.id, &p.data[i], metas),
            &user,
        );
    }
    cu
}

#[test]
fn compare_compute_units() {
    // Average over many wallets: `find_program_address` cost depends on the bump.
    const N: u64 = 64;
    let progs = programs();
    let totals: Vec<[u64; 4]> = progs
        .iter()
        .map(|p| {
            let mut svm = setup(p);
            let mut t = [0u64; 4];
            for _ in 0..N {
                for (i, cu) in run(&mut svm, p).into_iter().enumerate() {
                    t[i] += cu;
                }
            }
            t.map(|x| x / N)
        })
        .collect();

    println!(
        "\n| Instruction | {} | {} | {} | Pinocchio vs no-log Anchor |",
        progs[0].name, progs[1].name, progs[2].name
    );
    println!("|---|---:|---:|---:|---:|");
    for (i, name) in ["initialize", "increment", "decrement", "close"]
        .iter()
        .enumerate()
    {
        let [a, b, c] = [totals[0][i], totals[1][i], totals[2][i]];
        println!(
            "| `{name}` | {a} | {b} | {c} | {:.0}% fewer |",
            100.0 * (b - c) as f64 / b as f64
        );
        assert!(c < b && b <= a, "{name}");
    }
    println!(
        "| Binary size | {} B | {} B | {} B | |",
        progs[0].so.len(),
        progs[1].so.len(),
        progs[2].so.len()
    );
    println!("\nAverage over {N} wallets; whole-transaction CU in LiteSVM 0.10.");
}
