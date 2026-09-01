# riffle miniapp

A Telegram Mini App for the riffle liquidity-farming tool. This is the only place a wallet's
private key ever exists: it is generated or imported here, encrypted at rest under a user
passphrase, and decrypted only for the moment a signature is needed. The backend builds unsigned
transactions and never sees, stores, or logs key material -- see "What the backend never gets"
below for the mechanisms that make that true rather than just asserted.

## Contents

- [Build and run](#build-and-run)
- [Custody model, in one paragraph](#custody-model-in-one-paragraph)
- [KDF and cipher parameters](#kdf-and-cipher-parameters)
- [The transaction verifier](#the-transaction-verifier)
- [What the backend never gets](#what-the-backend-never-gets)
- [HTTP contract expected from the backend](#http-contract-expected-from-the-backend)
- [Dependencies, and why each one is here](#dependencies-and-why-each-one-is-here)
- [Storage and recovery](#storage-and-recovery)
- [Local development against Telegram](#local-development-against-telegram)
- [Deployment](#deployment)
- [Testing](#testing)

## Build and run

Requires Node 20+ and npm.

```sh
cd miniapp
npm install          # once; installs pinned versions from package-lock.json
cp .env.example .env.local
npm run typecheck    # tsc --noEmit, strict mode, zero errors expected
npm test             # vitest, unit tests only, no network or browser needed
npm run build         # type-checks, then produces dist/ via vite build
npm run dev            # local dev server on :5173
```

`npm run build` runs `tsc --noEmit` before `vite build` -- a type error fails the build, it is
never silently bypassed by Vite's own faster transpile-only path.

## Custody model, in one paragraph

A wallet is a BIP-39 mnemonic (or, for the secondary import path, a raw secret key), generated or
imported entirely client-side, immediately encrypted with a passphrase-derived key, and written to
this origin's IndexedDB as ciphertext, salt, nonce, and KDF parameters -- never the passphrase,
never the plaintext. There is no persistent "unlocked" session: every signing action decrypts the
key fresh, uses it to sign one transaction, and lets the reference go. The backend builds every
transaction the app is asked to sign, which means the app cannot take backend output on faith --
see "The transaction verifier" for the control that makes that safe.

## KDF and cipher parameters

**KDF: Argon2id**, via `libsodium-wrappers-sumo`'s `crypto_pwhash` (`src/crypto/kdf.ts`).
Memory-hard, which is what makes it resistant to GPU/ASIC-parallel brute force against a stolen
ciphertext in a way PBKDF2 or bcrypt are not -- the same amount of "work" for a legitimate device
doing it once is far more expensive to parallelize at scale than a plain-hash-iteration KDF.

Parameters, chosen as an OWASP mobile-friendly starting point rather than a desktop-tuned one,
because a phone running inside Telegram's in-app browser is the binding constraint:

| Parameter | Value | Why |
|---|---|---|
| Memory | 64 MiB | Large enough to be costly to parallelize in hardware, small enough not to stall or crash a mid-range phone's browser tab. |
| Iterations (`opslimit`) | 3 | Targets roughly one second of wall-clock cost on a mid-range Android device -- enough friction to matter against offline brute force, not so much that unlocking the app feels broken. |
| Parallelism | 1 (fixed by libsodium's Argon2id implementation) | Correct for a browser tab, which has no reliable multi-threading guarantee to spend a higher parallelism factor on productively. |
| Salt | 16 random bytes per vault, from `crypto.getRandomValues` | Standard Argon2id salt size; makes precomputed rainbow-table-style attacks against a single ciphertext pointless. |

These are a starting point, not a permanent constant: if real usage on real devices shows this
running uncomfortably slow, `opslimit` is the parameter to adjust first (raising `memlimit` is more
disruptive, since every previously-written vault's stored `kdf` parameters must keep decrypting
correctly -- see the `version` field on the stored blob).

**Cipher: XChaCha20-Poly1305** (IETF variant), via the same library's
`crypto_aead_xchacha20poly1305_ietf_encrypt`/`_decrypt` (`src/crypto/cipher.ts`). Chosen over
AES-256-GCM specifically for its 24-byte extended nonce: GCM's 12-byte nonce makes accidental
nonce reuse across two encryptions under the same key a real risk if a future passphrase-change
feature is implemented carelessly, and nonce reuse breaks GCM's confidentiality outright.
XChaCha20's 192-bit nonce space makes a random collision practically impossible even across many
re-encryptions, so "always generate a fresh random nonce" (which this code does, every time,
`randomNonce()`) is sufficient on its own with no separate counter bookkeeping to get wrong.

Both are implemented by libsodium's compiled-to-WASM build, audited and widely deployed, not
hand-rolled -- and pinned to an exact version (`libsodium-wrappers-sumo@0.8.4`) rather than a
range, so a dependency update to the actual cryptographic primitives is always a reviewed,
deliberate step.

**What gets encrypted**: for a mnemonic-backed wallet, the BIP-39 entropy (16 or 32 bytes,
depending on word count) -- not the full sentence, since the sentence is a lossless, deterministic
function of the entropy and storing both would just be redundant plaintext-shaped surface. For the
secondary raw-secret-key import path, the 64-byte secret key directly. See `src/crypto/vault.ts`.

**What never gets stored, anywhere, under any circumstance**: the passphrase itself, and the
decrypted key outside the lifetime of one signing operation. `src/crypto/memory.ts` documents,
without overclaiming, what "wipe after use" can and cannot guarantee in a garbage-collected
runtime -- worth reading directly rather than summarized here, since overclaiming this specific
point is the kind of thing worth being exact about.

## The transaction verifier

`src/verify/txVerifier.ts` is the single most load-bearing file in this app. The backend builds
every transaction; this app treats that output as untrusted and independently decodes the raw
bytes it is about to sign, checking them against the same summary rendered on screen
(`src/verify/fromSummary.ts` turns that summary into the expectation checked against). Any
mismatch is a hard failure -- there is no "sign anyway" path anywhere in the codebase.

What it checks, for every transaction, before ever prompting for a passphrase:

1. **No address lookup tables.** A transaction using one is refused outright, not partially
   resolved -- an ALT loads extra accounts from an on-chain table this app has not read, and
   trusting it would mean trusting the same untrusted source (the backend, or its RPC) that named
   the table in the first place. None of this app's flows need one.
2. **Every instruction's program id is on a fixed allow-list**: the DLMM program, ComputeBudget,
   System, SPL Token, Token-2022, and the associated-token-account program -- nothing else, ever
   (`src/solana/constants.ts`'s `ALLOWED_PROGRAM_IDS`).
3. **Every required signer is accounted for.** The only accounts a transaction may require a
   signature from are this wallet's own pubkey and, for opening a position, the client-generated
   ephemeral position keypair. A transaction that requires any other signer is refused, and a
   transaction that does *not* require this wallet's own signature at all is refused too (a sign
   something is being routed around the check entirely).
4. **Exactly one DLMM instruction**, identified by its Anchor discriminator (computed at
   verification time via `sha256("global:<name>")`, not hard-coded as a byte literal, so the check
   stays self-verifying against its own description), and it must be the instruction the approved
   summary's action kind implies -- `add-liquidity` must compile to `add_liquidity_by_strategy2`,
   never anything else, on a mismatch either way.
5. **Every account in that instruction is independently re-derived and compared**, not trusted as
   given: the position account, the pool, both token mints and their token programs, both of this
   wallet's own associated token accounts (re-derived from owner+mint+token-program, the standard
   SPL formula), both pool reserve accounts, every bin-array PDA the operation's range touches, the
   optional bitmap-extension account, and the fixed event-authority PDA. Every one of these
   derivations mirrors the account list a correct backend instruction-builder produces, seed for
   seed (see the account-order comments inline in `txVerifier.ts` and `src/solana/pda.ts`).
6. **Every decoded argument is compared against what was shown**: deposit/withdrawal amounts (with
   an optional caller-supplied basis-point tolerance for quoted-vs-executed rounding, zero by
   default -- exact match), bin ranges, the active-bin slippage bound, the withdrawal fraction, and
   the position's own range bounds (an operation whose range exceeds the position it targets is
   refused even if every account matches, mirroring the same check the backend's own instruction
   builder is expected to make).
7. **ComputeBudget instructions**, if present, must be exactly `SetComputeUnitLimit` or
   `SetComputeUnitPrice`, and a price above a caller-configured ceiling is refused as an advisory
   check (see the HTTP contract section for why this specific cap cannot be a real security
   control).
8. **Associated-token-account creation instructions**, if present, must create an ATA for this
   wallet's own pubkey against one of the two expected mints -- nothing else.

**What this deliberately does not catch**, stated in the file's own module comment rather than
left implied:

- A pool that is legitimately what it claims to be but is itself a bad trade (a manipulated active
  bin, a rug-pull token). This checks the transaction matches the summary, not that the summary
  describes a good decision.
- SOL-wrapping instructions (a `System::transfer` into a wrapped-SOL token account plus
  `Token::syncNative`) are not recognized at all. A build response containing them fails closed as
  an unrecognized instruction. This is a deliberate scope decision, not an oversight: see the HTTP
  contract section below for what it means for the backend.
- Anything about a transaction being delayed, censored, or replaced after this app hands a signed
  copy off for submission -- that is a submission-layer concern (idempotency, below), not something
  a pre-sign byte check can see.
- A bug in the verifier itself. Passing this check is strong evidence, not proof; it is ordinary
  code with ordinary code's failure modes.

The app also **re-simulates independently**, against its own RPC connection
(`src/solana/connection.ts`, deliberately configurable to a different endpoint than whatever the
backend uses), immediately before prompting for a passphrase -- closing the gap where the
backend's bundled simulation result, or the RPC it used to produce one, cannot be trusted as the
sole gate. And it **re-checks blockhash expiry** a second time immediately before submission, not
only at review time, since entering a passphrase can take long enough for a blockhash to go stale
in between.

Test coverage for all of this lives in `tests/txVerifier.test.ts`, including the specific case the
milestone this work supports asks for by name: a transaction whose decoded bytes do not match the
approved summary (an altered deposit amount, a substituted pool, a redirected token destination, a
different instruction entirely, an unexpected extra signer, and an address-lookup-table
transaction), each asserted to be refused without ever reaching the point of prompting for a
passphrase.

## What the backend never gets

Enforced structurally, not just documented:

- **No field in the API surface can carry it.** Every request/response type in
  `src/api/types.ts` -- register-pubkey, every build-tx variant, submit, balances, positions -- has
  no key-material-shaped field anywhere in it. `src/api/client.ts` has exactly one `fetch` call
  site in the entire codebase; grepping it (or the whole `src/` tree) for `secretKey`, `entropy`,
  or `mnemonic` finds nothing near it.
- **No logging, anywhere.** There is no `console.*` call in `src/`. Nothing about a build request,
  a signed transaction, or a passphrase is ever written to a log, a report, or analytics.
- **Telegram `initData` is never trusted locally.** `src/telegram/webapp.ts` reads the raw string
  Telegram attaches to the launch and forwards it as a header on every request; this app never
  parses it for an access-control decision. The backend recomputing the HMAC (keyed on the bot
  token, which this app never has) and checking `auth_date` freshness is assumed to be the actual
  authentication boundary -- see the HTTP contract's `X-Telegram-Init-Data` note.

## HTTP contract expected from the backend

Nothing on the backend exists yet. This is the interface it is expected to be built against --
see `src/api/types.ts` and `src/api/client.ts` for the exact TypeScript shapes.

Every request carries the raw Telegram launch payload as a header, never in the body:

```
X-Telegram-Init-Data: <raw initData string>
```

The backend must recompute and verify this HMAC and check `auth_date` freshness on every request;
this app treats it as opaque and never validates it locally.

| Method | Path | Purpose |
|---|---|---|
| POST | `/api/v1/wallet/register` | Registers this device's pubkey against the Telegram identity in `initData`. The only place a public key ever reaches the backend. |
| GET | `/api/v1/wallet/balances` | SOL and SPL/Token-2022 balances for the registered pubkey, read live from chain -- not cached as a source of truth. |
| GET | `/api/v1/positions` | Open (and recently closed) real positions for the registered pubkey. |
| POST | `/api/v1/tx/open-position` | Builds `initialize_position2`. Takes the client-generated ephemeral position pubkey (see note below) as an input, never generates or signs it. |
| POST | `/api/v1/tx/add-liquidity` | Builds `add_liquidity_by_strategy2`, `SpotBalanced` strategy only at launch. |
| POST | `/api/v1/tx/remove-liquidity` | Builds `remove_liquidity_by_range2`. |
| POST | `/api/v1/tx/claim-fees` | Builds `claim_fee2`. |
| POST | `/api/v1/tx/close-position` | Builds `close_position2`. |
| POST | `/api/v1/tx/submit` | Relays an already-signed transaction to RPC opaquely -- see "who submits" below. |
| GET | `/api/v1/tx/status` | Polls a previously submitted signature's status. |

**Every build-tx response must include**, alongside the unsigned transaction bytes:
`expiryBlockhash`/`expiryLastValidBlockHeight`, an `idempotencyKey`, a `simulation` result, an
estimated network fee, and a `summary` -- the exact object the review screen renders and the
verifier checks the transaction against (`src/verify/fromSummary.ts`). The summary's fields are
deliberately more complete than a minimal display would need (token program ids, the position's
own bin range) specifically so the verifier has one single source of "what the user was told" to
check against, rather than reconstructing part of its expectation from elsewhere.

**The ephemeral position key** (`open-position` only): `initialize_position2` needs two signers,
`payer` and `position`, where `position` is a fresh keypair that becomes the new position
account's own address, not a PDA. This app generates that keypair client-side and sends only its
public half in the build request; the backend references it in the instruction it builds but never
sees or signs with its private half. Keeps "the backend never signs anything, ever" an absolute
statement with no footnoted exception for a low-value throwaway key.

**Who submits**: the backend, as a thin relay that forwards already-signed raw bytes to RPC
opaquely, without inspecting or altering them (`POST /tx/submit`). This was chosen over the app
holding its own RPC connection to submit directly, specifically to avoid exposing a paid RPC
provider key client-side, while keeping "the backend never signs, never alters after signing"
true end to end -- the backend receiving signed bytes is not the same as the backend being able to
produce a valid signature itself.

**Idempotency**: every build request carries a client-generated `idempotencyKey` covering the
action and its parameters. A retried build under the same key must return the same transaction, or
-- if a prior build under that key has already confirmed on chain -- a short-circuit response
naming the existing signature instead of building a second, different transaction against a fresh
blockhash. Solana's own dedup only protects an *identical* signed transaction from landing twice;
it does nothing for two different-but-equivalent transactions built moments apart, which is the
real shape a naive retry-on-timeout takes.

**No SOL-wrapping instructions in a build response**, by contract, not just by the verifier's
current scope: if a flow needs a wrapped-SOL token account, the backend must assume it already
exists rather than including a `System::transfer` + `Token::syncNative` pair in the built
transaction. The verifier does not recognize that instruction shape and will refuse to sign a
transaction that includes it (see "What this deliberately does not catch" above) -- this is a
contract decision made once, here, rather than the verifier guessing whether a wrap amount looks
"close enough" to what was shown.

**Per-user notional caps**: the backend cannot enforce one by refusing to build over a limit --
this custody model means the backend never holds a key capable of refusing to *sign*, and a build
it declines to construct, a modified copy of this app could construct itself from the same public
account data. Any cap the backend applies here is advisory UI copy at best, and
`maxComputeUnitPriceMicroLamports` in `VerificationContext` is the same kind of advisory check, not
a security control -- worth being exact about so it is never presented to an operator as
protection it cannot provide.

## Dependencies, and why each one is here

Every package listed here can, in principle, read a decrypted key while it is in memory during a
signing operation -- that is the whole reason this list stays short and each entry is justified
rather than added by convenience. All versions are pinned exactly (no `^`/`~`) in `package.json`,
backed by `package-lock.json`.

**Runtime:**

- **`@solana/web3.js`** -- the reference implementation for Solana's wire format
  (`VersionedTransaction`, `Message`/`MessageV0`), account/keypair types, and RPC. Unavoidable for
  a Solana app that must decode transaction bytes byte-for-byte; a hand-rolled parser here would be
  strictly more risk for no benefit, since decoding correctness is exactly what the verifier's
  safety depends on.
- **`@scure/bip39`** -- BIP-39 mnemonic generation, validation, and entropy conversion. Part of the
  `@noble`/`@scure` ecosystem: audited, minimal-dependency, widely used in wallet software
  specifically because of that track record.
- **`@noble/hashes`** -- HMAC-SHA512 (for the hand-rolled SLIP-0010 derivation,
  `src/crypto/slip10.ts`) and SHA-256 (for computing Anchor instruction discriminators at
  verification time, `src/solana/constants.ts`). Also a transitive dependency of `@scure/bip39`;
  taking it as a direct, exactly-pinned dependency means its version is a deliberate choice here,
  not whatever range a transitive resolution happens to pick.
- **`libsodium-wrappers-sumo`** -- Argon2id and XChaCha20-Poly1305, see the KDF/cipher section
  above. The "sumo" build specifically, not the smaller default `libsodium-wrappers`, because the
  default build excludes `crypto_pwhash` and this cipher construction to stay small -- confirmed
  directly against the installed package rather than assumed.
- **`bs58`** -- base58 encode/decode for the secondary raw-secret-key import/no-mnemonic path.
  Already a transitive dependency of `@solana/web3.js`; taken directly for the same
  pin-it-deliberately reason as `@noble/hashes`.
- **`react` / `react-dom`** -- the UI layer. No state-management, routing, or component-kit
  dependency was added on top: the app's navigation is a handful of conditionally rendered screens
  driven by plain `useState`/`useContext` (`src/state/WalletContext.tsx`), which does not need a
  router or a store library at this size.

**Explicitly not depended on, and why:**

- **zxcvbn** (or any dictionary-based passphrase-strength library) -- meaningfully better scoring
  than the length/charset heuristic in `src/crypto/passphraseStrength.ts`, but ships several
  hundred KB of wordlists for one screen's input validation, in a bundle whose entire justification
  for staying small is that every dependency in it can read the user's key. The heuristic's real
  limitations are documented in that file rather than hidden behind a false sense of rigor.
- **A generic borsh library** -- the five DLMM instruction argument layouts this app ever decodes
  are small and fixed (`src/verify/decode.ts`); a ~150-line hand-rolled byte-cursor reader is more
  auditable here than a general deserializer would be for five shapes.
- **@noble/curves / a dedicated SLIP-0010 package** -- `@solana/web3.js`'s own `Keypair.fromSeed`
  already does ed25519 key expansion from a 32-byte seed (via its own audited `tweetnacl`
  dependency); the only piece actually missing was the hierarchical derivation itself, which is
  ~30 lines of HMAC-SHA512 against a public spec (verified against the spec's own published test
  vectors in `tests/slip10.test.ts`), not enough to justify a new dependency.
- **A router, a state-management library, a component kit** -- see above.

**A known transitive advisory**: `@solana/web3.js`'s `rpc-websockets` dependency pulls in an old
`uuid` version with a moderate advisory (`npm audit`) in a code path this app's usage never
reaches (subscription/notification internals this app does not use, since balances and positions
are read via plain HTTP-backed RPC calls, not websocket subscriptions). Tracked here rather than
silently ignored; revisit when `@solana/web3.js` picks up a fixed `rpc-websockets`.

## Storage and recovery

The encrypted vault lives in this origin's IndexedDB (`src/storage/idb.ts`), not Telegram's
`CloudStorage` -- deliberately, so that a compromised Telegram account does not also hand over the
ciphertext (`CloudStorage` syncs through Telegram's own servers; IndexedDB does not). The trade
this accepts: no multi-device sync, and clearing site data or losing the device loses the
ciphertext permanently. There is no backend-mediated recovery of any kind -- no password reset, no
support-initiated restore -- because the backend never had a copy of the key to restore from. This
is stated plainly in the export-flow UI copy (`src/components/ExportKey.tsx`) rather than
softened: if both the passphrase and an exported backup phrase are lost, the funds are gone,
permanently.

`src/state/WalletContext.tsx` distinguishes "no wallet was ever created here" from "storage itself
is unavailable" (e.g. a private-browsing context that disables IndexedDB) -- the two look similar
to a user but need different UI copy, and the app never conflates them into a generic error.

Export (`src/components/ExportKey.tsx`) is the only way to get the recovery phrase back out: it
requires the passphrase, displays the phrase once, and clears it from the screen automatically
after sixty seconds or on demand. It carries an explicit warning that any export path is also an
exfiltration path -- there is no way to make legitimate recovery convenient without a phished or
malicious copy of this app finding the same screen equally convenient, and the only real levers are
friction and the passphrase requirement, not a cleverer UI.

## Local development against Telegram

1. `npm run dev` starts a local Vite server, by default on `http://localhost:5173`.
2. Telegram Mini Apps must be served over HTTPS from a domain registered with BotFather; for local
   iteration against a real Telegram client, tunnel the dev server (e.g. `ngrok http 5173`) and set
   that tunnel URL as the Mini App's URL via `@BotFather` → your bot → Bot Settings → Menu Button
   (or Mini App), which only needs to be updated when the tunnel URL changes.
3. Outside of an actual Telegram client, `window.Telegram.WebApp` is undefined --
   `src/telegram/webapp.ts` treats this as normal (`getInitData()` returns an empty string,
   `initTelegramApp()` is a no-op) so the rest of the UI still runs for local iteration; a real
   backend will reject an empty/invalid `initData`, so testing the registration and build-tx flows
   end-to-end still requires launching through Telegram itself, or pointing `VITE_API_BASE_URL` at
   a local backend stub that skips HMAC verification for development only.
4. Set `VITE_SOLANA_RPC_URL` to a devnet endpoint for any real signing/submission testing --
   never point a local dev environment at a mainnet RPC with funds behind it.

## Deployment

This is a static single-page bundle (`vite build` → `dist/`) with no server-side component of its
own -- deploy it to any static host that serves over HTTPS from the exact domain registered with
BotFather (Telegram enforces this at the platform level; a Mini App served from an unregistered
domain will not open inside Telegram). Treat the deploy pipeline and hosting for this bundle with
at least the operational care given the backend -- restricted deploy access, reviewed changes --
arguably more, since this is the actual signing surface a user's passphrase and key touch, not
merely a data plane. A compromised build or deploy pipeline for this bundle is a strictly worse
outcome than a compromised backend: see `src/verify/txVerifier.ts`'s module comment for why a
malicious copy of this app has no verifier left to catch it, since a compromised Mini App *is* the
verifier.

## Testing

```sh
npm test
```

Runs under `vitest` in a plain Node environment -- no browser, no DOM, no network. Covers:

- **Encryption round trip** (`tests/vault.test.ts`): both wallet-secret shapes encrypt and decrypt
  back to identical bytes under the right passphrase; the persisted blob's own key set is asserted
  to be exactly the fields the design allows (`ciphertext`, `createdAt`, `kdf`, `nonce`,
  `publicKey`, `salt`, `version`) and nothing else.
- **Wrong-passphrase failure** (`tests/vault.test.ts`, `tests/wallet.test.ts`): decryption under an
  incorrect passphrase throws rather than returning partial or garbage plaintext; a
  tampered-ciphertext blob fails the same way (AEAD authentication failure and a wrong passphrase
  are deliberately indistinguishable from the caller's side, so neither leaks which case occurred).
- **SLIP-0010 derivation** (`tests/slip10.test.ts`): checked against the published spec's own
  ed25519 test vector, independent of this project.
- **Mnemonic round trip and validation** (`tests/mnemonic.test.ts`): generation produces a valid
  phrase at both supported word counts; entropy↔mnemonic is lossless; a broken-checksum phrase is
  rejected.
- **The transaction verifier** (`tests/txVerifier.test.ts`): a correctly built transaction is
  accepted; an altered amount, a substituted pool, a redirected token destination, a wrong owner, a
  wrong instruction entirely, an unexpected extra required signer, and an address-lookup-table
  transaction are each refused -- the specific "decoded transaction does not match the displayed
  summary, and signing is refused" case this work was asked to demonstrate.
