# 03-pinocchio-counter

[![CI](https://github.com/malharc373/solana-pinocchio-counter/actions/workflows/ci.yml/badge.svg)](https://github.com/malharc373/solana-pinocchio-counter/actions/workflows/ci.yml)

My [Anchor counter](https://github.com/malharc373/solana-counter) rewritten in **[Pinocchio](https://github.com/anza-xyz/pinocchio) 0.12**: same behaviour, same PDA scheme, every check Anchor did now written by hand. The benchmark runs both builds through the same flow.

| | |
|---|---|
| Program ID (devnet) | [`DL4b35FZzH8ZnA8Nx6D67FxrnMrjYL5YHymhm3LWnFG1`](https://explorer.solana.com/address/DL4b35FZzH8ZnA8Nx6D67FxrnMrjYL5YHymhm3LWnFG1?cluster=devnet) |
| Example `increment` (331 CU) | [`5NdrZu…9Lh`](https://explorer.solana.com/tx/5NdrZukBDHhobdCVZL9AdxvtQsLwPNNsgpuCChWsdxc1smCGwMDFuGVY4YeBqzuwnhGSMb9kvsLS47ix9dJaR9Lh?cluster=devnet) |
| Example `close` (345 CU) | [`3Y83iE…RjAS`](https://explorer.solana.com/tx/3Y83iE2roQKRWR2crqPqAkc5DSriUU6r6bYqfqCyEggk13RqXsCKbUV3oxnAGDr5JdNAEj4VMLJQXaoc3q7RRjAS?cluster=devnet) |

## Results

```bash
cargo build-sbf && cargo test --test bench -- --nocapture
```

| Instruction | Anchor 1.1.2 | Anchor, no logs | Pinocchio 0.12 | Pinocchio vs no-log Anchor |
|---|---:|---:|---:|---:|
| `initialize` | 9,798 | 8,101 | 4,363 | 46% fewer |
| `increment` | 4,011 | 3,520 | 331 | 91% fewer |
| `decrement` | 4,035 | 3,523 | 323 | 91% fewer |
| `close` | 3,681 | 3,471 | 345 | 90% fewer |
| Binary size | 165,064 B | 156,152 B | 10,520 B | 15× smaller |

These are whole-transaction compute units in LiteSVM, averaged over 64 wallets. Devnet gives the same figures for `increment`, `decrement` and `close`. The cost of `initialize` depends on the wallet, because `find_program_address` pays about 1,500 CU for each bump it rejects.

**The comparison is fair.** The Anchor program logs on every instruction and this one logs nothing. So the middle column is the same Anchor commit with every `msg!` removed, built with `--features no-log-ix-name`. Both `.so` files are in [`bench/`](bench).

### Where the savings come from
- **The PDA check.** Anchor's `seeds = [...], bump = counter.bump` calls `create_program_address`, which costs 1,500 CU. Pinocchio's `Address::derive_address` does a single `sha256` with the stored bump, which is safe because that bump came from `find_program_address` when the account was created.
- **No deserialization.** Anchor copies the account into a struct and writes it back with Borsh. Here the 42 bytes are read and written in place.
- **Smaller entrypoint.** Pinocchio's entrypoint parses at most 3 accounts as zero-copy views (`program_entrypoint!(process_instruction, 3)`), with no allocator and no `std`.
- **1-byte instruction tags** instead of 8-byte discriminators.

## What Anchor did that this program does by hand

| Check | Anchor | Here | Test that fails without it |
|---|---|---|---|
| The account is ours | `Account<'info, Counter>` | `check_owner` | `rejects_a_forged_counter_owned_by_another_program` |
| Only the authority can change it | `has_one = authority` | stored authority == signer | `attacker_cannot_touch_someone_elses_counter` |
| The authority signed | `Signer<'info>` | `is_signer()` | `authority_must_sign` |
| Right PDA | `seeds`, `bump` | `find_program_address` / `derive_address` | `initialize_rejects_a_wrong_address` |
| No re-initialization | `init` | owner check before any CPI | `cannot_initialize_twice` |
| A pre-funded PDA can't block `init` | `init` falls back to transfer + allocate + assign | `create_pda_account` does the same | `initialize_survives_a_prefunded_pda` |
| Close refunds the rent | `close = authority` | move lamports, then `close()` | `close_refunds_rent_and_allows_a_fresh_start` |

**Mutation-tested:** I deleted each check in turn and confirmed that the matching test fails.

**The pre-funded PDA attack.** A PDA address is public before the account exists. If anyone sends it 1 lamport, a plain `create_account` CPI fails with "account already in use", and the user can never create their counter. Anchor handles this for you. A hand-written program has to check `lamports() > 0` and then top up, `allocate` and `assign` instead.

## Account layout (42 bytes)

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | discriminator (`1`) |
| 1 | 32 | authority |
| 33 | 8 | count, u64 LE |
| 41 | 1 | bump |

Instructions are `[tag]`: `0` initialize `[authority (w, s), counter (w), system_program]`, `1` increment and `2` decrement `[counter (w), authority (s)]`, `3` close `[counter (w), authority (w, s)]`. Errors are `Custom(0)` Unauthorized, `Custom(1)` Overflow and `Custom(2)` Underflow, the same as the Anchor version.

## Run it

```bash
cargo build-sbf
```

```bash
cargo test
```

Devnet demo (initialize → increment ×2 → decrement → close, printing CU per transaction):

```bash
cd client && npm install && npm run demo
```

## What I learned
- **Pinocchio 0.12 assumes the new rent sysvar layout (SIMD-0194).** `Rent::get()` reads only the first field as lamports per byte. Devnet and mainnet already use that layout: `5080` per byte with threshold `1.0`, which I read from the sysvar on 2026-11-23. **LiteSVM 0.10 still seeds the old layout** (`3480` per byte-year, threshold `2.0`), so Pinocchio asks for half the rent and `initialize` fails with `InsufficientFundsForRent`. LiteSVM 0.18 fixes this but pins `solana-address ~2.7`, which conflicts with Pinocchio 0.12's `^2.8`. The tests therefore set the rent sysvar to the live values.
- **Logs cost real CU.** Anchor's instruction-name log and one formatted `msg!` add about 500 CU to a 3,500 CU instruction.
- **`create_program_address` costs 1,500 CU.** Storing the canonical bump and re-deriving with plain `sha256` is the single biggest saving for any program that re-checks PDAs.
- **Most of Anchor's safety is a handful of `if` statements.** They are easy to forget, so each one gets a test that fails when it is deleted.
