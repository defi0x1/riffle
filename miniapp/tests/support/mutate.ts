import bs58 from "bs58";
import { Message, PublicKey, TransactionInstruction, VersionedTransaction } from "@solana/web3.js";

import { DLMM_PROGRAM_ID } from "../../src/solana/constants";

/**
 * Byte-realistic negative fixtures: every function here starts from a genuine, Rust-built
 * transaction (loaded via support/fixtures.ts) and mutates it structurally -- swapping which
 * pubkey occupies an account slot, flipping a bit inside an instruction's argument bytes,
 * appending a duplicate instruction -- rather than hand-assembling a new transaction from
 * scratch. That keeps every negative case anchored to real backend output instead of to this
 * test file's own idea of what the wire format looks like.
 *
 * All of this only works because these fixtures are legacy (non-versioned) messages: a legacy
 * `Message`'s `accountKeys` and `instructions` arrays are the actual mutable state `serialize()`
 * reads from (unlike `compiledInstructions`, a read-only view computed fresh on every access).
 */

function requireLegacyMessage(tx: VersionedTransaction): Message {
  if (tx.message.version !== "legacy") {
    throw new Error("mutation helpers only support legacy-message fixtures");
  }
  return tx.message;
}

function applyMutation(bytes: Uint8Array, mutate: (message: Message) => void): Uint8Array {
  const tx = VersionedTransaction.deserialize(bytes);
  const message = requireLegacyMessage(tx);
  mutate(message);
  return new VersionedTransaction(message, tx.signatures).serialize();
}

function dlmmInstructionIndex(message: Message): number {
  const idx = message.instructions.findIndex((ix) =>
    message.accountKeys[ix.programIdIndex]?.equals(DLMM_PROGRAM_ID),
  );
  if (idx === -1) {
    throw new Error("fixture has no DLMM instruction to mutate");
  }
  return idx;
}

/** Flips every bit of one byte inside the DLMM instruction's argument data (offset counted from
 * the start of the data, i.e. byte 0 is the first byte of the 8-byte discriminator). Guaranteed
 * to change the decoded value, since XOR 0xff can never be a no-op. */
export function flipDlmmInstructionByte(bytes: Uint8Array, byteOffset: number): Uint8Array {
  return applyMutation(bytes, (message) => {
    const idx = dlmmInstructionIndex(message);
    const ix = message.instructions[idx];
    if (!ix) throw new Error("instruction index out of range");
    const data = bs58.decode(ix.data);
    if (byteOffset >= data.length) {
      throw new Error(`byte offset ${byteOffset} is beyond instruction data length ${data.length}`);
    }
    const current = data[byteOffset];
    if (current === undefined) throw new Error("byte offset out of range");
    data[byteOffset] = current ^ 0xff;
    ix.data = bs58.encode(data);
  });
}

/** Replaces the first occurrence of `from` anywhere in the message's account list with `to`,
 * leaving that slot's signer/writable position (and everything else) untouched -- models a
 * backend that redirects one specific account (the pool, the owner, ...) to something else while
 * leaving the rest of the transaction looking exactly as the user was shown. */
export function substituteAccount(bytes: Uint8Array, from: PublicKey, to: PublicKey): Uint8Array {
  return applyMutation(bytes, (message) => {
    const idx = message.accountKeys.findIndex((key) => key.equals(from));
    if (idx === -1) {
      throw new Error(`account ${from.toBase58()} is not present in this fixture`);
    }
    message.accountKeys[idx] = to;
  });
}

/** Swaps the pubkey *values* at two existing account slots, keeping every slot's signer/writable
 * position fixed. Used to model "changed signer": swapping the wallet's slot with an unrelated
 * already-present account's slot means the message now requires a signature from that other
 * account's key instead of the wallet's -- an unexpected-signer requirement that arises from
 * legitimate-looking bytes, not a value that never appeared in the transaction at all. */
export function swapAccounts(bytes: Uint8Array, a: PublicKey, b: PublicKey): Uint8Array {
  return applyMutation(bytes, (message) => {
    const idxA = message.accountKeys.findIndex((key) => key.equals(a));
    const idxB = message.accountKeys.findIndex((key) => key.equals(b));
    if (idxA === -1 || idxB === -1) {
      throw new Error("one or both accounts are not present in this fixture");
    }
    const valueA = message.accountKeys[idxA];
    const valueB = message.accountKeys[idxB];
    if (!valueA || !valueB) throw new Error("account index out of range");
    message.accountKeys[idxA] = valueB;
    message.accountKeys[idxB] = valueA;
  });
}

/** Overwrites the DLMM instruction's 8-byte discriminator with a different one, leaving every
 * account and every argument byte after it untouched -- models a backend that builds the wrong
 * DLMM instruction entirely while keeping an otherwise-plausible account list. */
export function swapDlmmDiscriminator(bytes: Uint8Array, discriminator: Uint8Array): Uint8Array {
  return applyMutation(bytes, (message) => {
    const idx = dlmmInstructionIndex(message);
    const ix = message.instructions[idx];
    if (!ix) throw new Error("instruction index out of range");
    const data = bs58.decode(ix.data);
    data.set(discriminator.subarray(0, 8), 0);
    ix.data = bs58.encode(data);
  });
}

/** Appends an exact duplicate of the DLMM instruction to the message. Every account it touches
 * already exists in the transaction, so this needs no new account-list entries -- it purely
 * tests the verifier's "exactly one DLMM instruction" check. */
export function duplicateDlmmInstruction(bytes: Uint8Array): Uint8Array {
  return applyMutation(bytes, (message) => {
    const idx = dlmmInstructionIndex(message);
    const ix = message.instructions[idx];
    if (!ix) throw new Error("instruction index out of range");
    message.instructions.push({ programIdIndex: ix.programIdIndex, accounts: [...ix.accounts], data: ix.data });
  });
}

/** Pulls the genuine DLMM instruction back out of a fixture as a `TransactionInstruction`, real
 * accounts (with their actual signer/writable flags) and real data bytes intact. Lets a test
 * recompile a fresh message around this one real instruction plus something new -- an unrelated
 * decoy instruction, an address-lookup-table message -- without ever hand-encoding the DLMM
 * instruction itself. */
export function extractDlmmInstruction(bytes: Uint8Array): TransactionInstruction {
  const tx = VersionedTransaction.deserialize(bytes);
  const message = requireLegacyMessage(tx);
  const idx = dlmmInstructionIndex(message);
  const ix = message.instructions[idx];
  if (!ix) throw new Error("instruction index out of range");
  const programId = message.accountKeys[ix.programIdIndex];
  if (!programId) throw new Error("program id account index out of range");
  const keys = ix.accounts.map((accountIndex) => {
    const pubkey = message.accountKeys[accountIndex];
    if (!pubkey) throw new Error("account index out of range");
    return {
      pubkey,
      isSigner: message.isAccountSigner(accountIndex),
      isWritable: message.isAccountWritable(accountIndex),
    };
  });
  return new TransactionInstruction({ programId, keys, data: Buffer.from(bs58.decode(ix.data)) });
}
