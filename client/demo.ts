// Devnet demo for the Pinocchio counter using @solana/kit.
//   npm run demo
// Runs initialize → increment ×2 → decrement → close for your wallet and
// prints the compute units of each transaction from the confirmed chain data.
// Env: RPC_URL (default devnet), KEYPAIR (default ~/.config/solana/id.json)

import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import {
  AccountRole,
  address,
  appendTransactionMessageInstructions,
  createKeyPairSignerFromBytes,
  createSolanaRpc,
  createTransactionMessage,
  fetchEncodedAccount,
  getAddressEncoder,
  getBase64EncodedWireTransaction,
  getProgramDerivedAddress,
  getSignatureFromTransaction,
  pipe,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
  type Address,
  type Instruction,
  type KeyPairSigner,
} from "@solana/kit";

const PROGRAM_ID = address("DL4b35FZzH8ZnA8Nx6D67FxrnMrjYL5YHymhm3LWnFG1");
const SYSTEM_PROGRAM = address("11111111111111111111111111111111");
const RPC_URL = process.env.RPC_URL ?? "https://api.devnet.solana.com";
const CLUSTER = RPC_URL.includes("devnet") ? "?cluster=devnet" : "";
const TAG = { initialize: 0, increment: 1, decrement: 2, close: 3 } as const;

const rpc = createSolanaRpc(RPC_URL);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Retries an RPC call when the public endpoint rate-limits us (HTTP 429). */
async function retry<T>(call: () => Promise<T>, attempts = 6): Promise<T> {
  for (let i = 1; ; i++) {
    try {
      return await call();
    } catch (e) {
      const status = (e as { context?: { statusCode?: number } }).context?.statusCode;
      if (status !== 429 || i >= attempts) throw e;
      await sleep(2000 * i);
    }
  }
}

async function send(payer: KeyPairSigner, ix: Instruction): Promise<[string, bigint]> {
  const { value: blockhash } = await retry(() => rpc.getLatestBlockhash().send());
  const tx = await signTransactionMessageWithSigners(
    pipe(
      createTransactionMessage({ version: 0 }),
      (m) => setTransactionMessageFeePayerSigner(payer, m),
      (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
      (m) => appendTransactionMessageInstructions([ix], m),
    ),
  );
  const sig = getSignatureFromTransaction(tx);
  await retry(() => rpc.sendTransaction(getBase64EncodedWireTransaction(tx), { encoding: "base64" }).send());
  // Poll over HTTP: the public devnet WebSocket drops connections.
  for (let i = 0; ; i++) {
    const { value: [status] } = await retry(() => rpc.getSignatureStatuses([sig]).send());
    if (status?.err) throw new Error(`${sig} failed: ${JSON.stringify(status.err, (_, v) => (typeof v === "bigint" ? `${v}` : v))}`);
    if (status?.confirmationStatus === "confirmed" || status?.confirmationStatus === "finalized") break;
    if (i >= 60) throw new Error(`${sig} not confirmed after 120s`);
    await sleep(2000);
  }
  const info = await retry(() =>
    rpc.getTransaction(sig, { commitment: "confirmed", encoding: "json", maxSupportedTransactionVersion: 0 }).send(),
  );
  return [sig, info?.meta?.computeUnitsConsumed ?? 0n];
}

function ix(tag: number, accounts: Instruction["accounts"]): Instruction {
  return { programAddress: PROGRAM_ID, data: new Uint8Array([tag]), accounts };
}

/** Raw 42-byte layout: [disc u8][authority 32][count u64 LE][bump u8] */
async function readCount(counter: Address): Promise<bigint | null> {
  const acc = await retry(() => fetchEncodedAccount(rpc, counter));
  if (!acc.exists) return null;
  return new DataView(acc.data.buffer, acc.data.byteOffset).getBigUint64(33, true);
}

// ---------------------------------------------------------------------------
const path = process.env.KEYPAIR ?? `${homedir()}/.config/solana/id.json`;
const user = await createKeyPairSignerFromBytes(new Uint8Array(JSON.parse(readFileSync(path, "utf8"))));
const [counter] = await getProgramDerivedAddress({
  programAddress: PROGRAM_ID,
  seeds: ["counter", getAddressEncoder().encode(user.address)],
});
console.log(`wallet  ${user.address}\ncounter ${counter}`);

const signer = { address: user.address, role: AccountRole.WRITABLE_SIGNER, signer: user };
const update = (tag: number) =>
  ix(tag, [
    { address: counter, role: AccountRole.WRITABLE },
    { ...signer, role: AccountRole.READONLY_SIGNER },
  ]);
const log = (label: string, [sig, cu]: [string, bigint]) =>
  console.log(`  ${label.padEnd(10)} ${String(cu).padStart(5)} CU  https://explorer.solana.com/tx/${sig}${CLUSTER}`);

if ((await readCount(counter)) === null) {
  log("initialize", await send(user, ix(TAG.initialize, [signer, { address: counter, role: AccountRole.WRITABLE }, { address: SYSTEM_PROGRAM, role: AccountRole.READONLY }])));
}
log("increment", await send(user, update(TAG.increment)));
log("increment", await send(user, update(TAG.increment)));
log("decrement", await send(user, update(TAG.decrement)));
console.log(`  count = ${await readCount(counter)}`);
log("close", await send(user, ix(TAG.close, [{ address: counter, role: AccountRole.WRITABLE }, signer])));
console.log(`  closed: ${(await readCount(counter)) === null}`);
