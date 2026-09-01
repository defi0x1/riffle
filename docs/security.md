# Security model

This describes the system as it exists in the source tree today, plus the shape it is being
built toward: a backend that indexes pools, ranks them, and -- as of the `wallets` /
`transaction_intents` schema and `dlmm_tx` instruction builders -- can construct an unsigned
Solana transaction on a user's behalf, but never signs one. Read this alongside
[`docs/architecture.md`](architecture.md) for how the pieces fit together and
[`docs/operations.md`](operations.md) for how the system is actually run; this document is
about what can go wrong and what stops it, not how the system works day to day.

## 1. What this system holds, and what it deliberately does not

**Holds:** pool state and swap/liquidity event history read from chain; computed indicators,
rankings, and signals with the inputs that produced them; a Telegram chat-id allow-list;
per-user wallet *public* keys (`wallets.pubkey`) and the on-chain positions and balances
associated with them; unsigned transactions the backend built (`transaction_intents.
unsigned_tx_base64`) together with their eventual signature and confirmation status, once the
device has signed and submitted them; operational credentials the backend itself needs to run
-- the Telegram bot token, an RPC/Geyser API key, the database connection string.

**Does not hold, anywhere, by construction:** a private key, a seed phrase, a mnemonic, a
passphrase, or an encrypted keystore blob belonging to a user. There is no column for any of
these in the schema (`migrations/0028_wallets.sql` and `0030_transaction_intents.sql` both
carry an explicit comment saying so, as a tripwire for whoever edits them next), no field for
one in any request or response type the backend exposes, and -- enforced, not just
documented -- no Rust identifier capable of holding or deriving one anywhere in backend source
(`scripts/keyless-guard.sh`). Key generation, encryption at rest, and signing all happen
on the user's own device, inside the Telegram Mini App -- `miniapp/`, a separate codebase and
toolchain (TypeScript, its own CI lane) living in this same repository, but not one this
guard's Rust-source scan reaches, and not one any check in section 3 other than the secret
scanner and the `miniapp` job itself looks at.

This is why `unsigned_tx_base64` is stored as plain text rather than something requiring
protection: a Solana transaction message with an empty signature slot cannot, by definition of
the wire format, contain a private key. The backend can leak every row in that table and an
attacker still cannot move a single lamport with it -- they can only see what was proposed.

## 2. Trust boundaries

```
   user's device                 this backend                  Solana
  (Mini App, holds        (indexes, ranks, builds       (source of truth for
   the private key)        unsigned transactions)        balances and positions)
        |                         |                              |
        | register pubkey ------>|                              |
        | request build -------->|                              |
        |<---- unsigned tx ------|                              |
        | independently verify   |                              |
        | the raw instruction    |                              |
        | bytes before signing   |                              |
        | sign, submit ------------------------------------------->|
        | poll status ------------------------------------------->|
        |<-------------- confirmed / failed -----------------------|
        | report signature ----->|                              |
```

Three boundaries matter, and they are not symmetric:

- **Device to backend.** The backend authenticates a request as coming from a real Telegram
  session (recomputing Telegram's own HMAC over the Mini App launch payload, the same
  choke-point shape `bot::auth::is_authorized` already uses for chat access), then checks the
  associated wallet is registered and not revoked. This defends against an unrelated third
  party calling the API; it does not, and cannot, defend against a backend that has itself been
  compromised -- see below.
- **Backend to device.** The backend can propose *anything* as an unsigned transaction and
  describe it however it likes. A compromised or buggy backend could build a transaction that
  drains a position to an attacker-controlled account while describing it as a routine claim.
  Nothing on the backend side stops this from being served; the only thing that stops it from
  being *signed* is the device independently decoding the raw instruction bytes -- program ids,
  signer accounts, destination token accounts, amounts -- and refusing to prompt for a
  passphrase if any of it disagrees with what it is about to show the user. That verification
  step lives entirely in `miniapp/`, which is the load-bearing fact in this whole design:
  **this backend cannot be made to hold a key, but it can be made to lie, and the only defense
  against that lie is client-side code running on a device this backend does not control.**
- **Device/backend to Solana.** Standard blockchain trust assumptions apply: a hostile or
  merely unreliable RPC endpoint can misreport simulation results, balances, or pool state, and
  can delay or drop a submission. It cannot forge a signature or alter an already-signed
  transaction without invalidating it. Use an RPC provider worth trusting for anything
  balance- or build-relevant; this is an operational choice, not something code can enforce.

**What this means for a user, concretely, not just for whoever built this:**

- **Never paste a private key, seed phrase, or recovery phrase into this chat, or into any
  chat, ever.** No legitimate flow here asks for one -- the Telegram bot's `/wallet` command
  only ever accepts a public key, and refuses anything shaped like key material without echoing
  it back or logging it (see [`docs/telegram.md`](telegram.md)'s "Never paste a key"). A key or
  phrase typed into a chat is compromised the moment it is sent, refused or not, because Telegram
  itself has already kept a copy in that chat's history.
- **Never share your Mini App passphrase with anyone**, including someone claiming to be able to
  help recover a wallet. There is no backend-mediated recovery of any kind -- no password reset,
  no support-initiated restore, because the backend never had a copy of the key to restore from
  (see `miniapp/README.md`'s "Storage and recovery"). A request for the passphrase is never a
  legitimate recovery step; it is the entire attack.
- **Never approve a transaction in the Mini App whose displayed summary you do not understand.**
  The summary is the one thing standing between what the backend proposed and what your
  signature actually authorizes -- the verifier checks the transaction matches the summary, not
  that the summary describes a good decision (see "The transaction verifier" in
  `miniapp/README.md`). If a pool, an amount, or an action does not look like what you asked for
  in the chat, stop and do not sign; treat an unfamiliar or unexplained summary the same way.

## 3. What the automated checks enforce

| Check | Job | Enforces | Does not catch |
|---|---|---|---|
| `scripts/keyless-guard.sh` | `keyless-guard` | Backend Rust source and manifests never reference a signing-capable type (`Keypair`, `Signer`, `SigningKey`, `SecretKey`) or key-material vocabulary (mnemonic, BIP39/BIP32/SLIP-10, passphrase, seed phrase, private/secret key naming) | Logic bugs that leak a *public* key or other non-secret data; anything client-side, since it only scans this repository |
| `scripts/provenance-check.sh` | `provenance` | Internal-only identifiers and references to the private planning repository never land in tracked source | Secrets -- it is a naming-leak guard, not a credential scanner |
| gitleaks (`.gitleaks.toml`) | `secrets` | Committed credentials, in the current tree and full commit history, against a real ruleset (not just this project's own vocabulary) | A secret introduced and never committed; a secret shaped unlike anything the ruleset or this project's allowlist anticipates |
| cargo-deny (`deny.toml`) | `dependencies` | Every dependency against RUSTSEC advisories (fails the build), license policy (warns), and known-bad/duplicate sources | A vulnerability that has not yet been reported upstream; a malicious *new* dependency someone adds and also adds a matching `deny.toml` exception for in the same change -- exceptions still want a second reviewer to read them, not just exist |
| `clippy -D warnings`, `test`, `integration` | `clippy`, `test`, `integration` | Compiles, lints clean, and the existing test suite (unit + a real Postgres) still passes | Anything the test suite does not exercise -- these are correctness gates, not security gates, though a broken build is its own kind of risk |
| `npm run typecheck`, `npm test`, `npm audit` | `miniapp` | The Mini App's own type safety, test suite, and dependency advisories, at the same `--audit-level=high` bar `deny.toml` applies on the Rust side | The two things that actually matter most for this design: whether the served bundle matches this reviewed source, and who can change what gets served. Checklist items below, not a CI job -- there is no automated way to verify a deployment target from inside the repository being deployed |

Each of these is a gate a pull request cannot get past silently, but `keyless-guard`,
`provenance`, `dependencies`, `clippy`, `test`, and `integration` are all Rust-source checks --
they do not read a line of `miniapp/`. Only `secrets` and `miniapp` reach it, and neither one
can confirm that the code reviewed here is the code actually served to a user's browser on
launch, or who besides CI can change that. That gap is not an oversight; a repository cannot
verify its own deployment from inside itself. It is exactly the part of the design that matters
most once real funds are involved, and it is why the checklist below treats it as the
headline item, not a footnote.

## 4. What remains a human responsibility

- **Reading `deny.toml`'s exceptions when they change.** The config comments explain why each
  currently-ignored advisory is believed safe to defer; a new one appearing in a diff is worth
  actually reading, not rubber-stamping, especially if the reasoning leans on "this backend
  never signs" -- that argument stops applying the moment signing code is ever added anywhere
  in this tree.
- **RPC/Geyser provider selection.** No automated check can tell a reputable provider from an
  unreliable or hostile one; this is an operational judgment call, revisited as the system's
  stakes change.
- **The Telegram chat allow-list and bot token custody.** Unchanged from the read-only system:
  the allow-list is still the entire access-control story for chat commands, and the bot token
  is still a bearer credential for the whole bot if it leaks, independent of anything in this
  document.
- **The parts of the Mini App no CI job can see.** `typecheck`/`test`/`npm audit` catch type
  errors, regressions, and known-vulnerable dependencies; they say nothing about whether the
  independent-verification step actually runs before every signature (a correctness property
  no test suite proves by existing), who has push/deploy access to wherever the built bundle is
  served, or whether what is served matches what was reviewed. See the checklist below --
  several items exist specifically because nothing automated, in this repository or otherwise,
  can confirm them from the outside.
- **Recognizing that a compromised backend is a real, differently-shaped threat than a
  compromised key.** A full backend compromise cannot move funds directly, but it can serve a
  plausible lie to every device that asks it to build a transaction. Backend hardening
  (secret rotation, least-privilege RPC credentials, a small audit surface, the checks in
  section 3) reduces the odds of that compromise happening; it does nothing once it has, which
  is worth remembering when deciding how much confidence any of this section 3 buys.

## 5. Pre-release audit checklist

Work through this before shipping any version that can move real funds -- i.e., before the
first release where `dlmm_tx`-built transactions reach a real device with a funded wallet. Each
item is something to run or check, with an expected result, not a box to tick from memory.

**This repository:**

1. Run `sh scripts/keyless-guard.sh` and `sh scripts/provenance-check.sh` from a clean
   checkout. Expect both to exit `0`.
2. Run `gitleaks detect --source . --config .gitleaks.toml -v`. Expect `no leaks found`. If it
   finds something, treat the credential as burned and rotate it even if the finding looks old
   -- history scanning exists because a removed secret is still a leaked one.
3. Run `cargo deny check advisories bans sources`. Expect exit `0`. Read every entry currently
   in `deny.toml`'s `[advisories].ignore` list and confirm each reason still holds against the
   dependency tree at release time, not just at the time it was written.
4. Confirm every endpoint that can build a transaction (`register-pubkey`, and one per action:
   open/add/remove/claim/close) is behind the same Telegram `initData` authentication and
   wallet-ownership check as every other per-user endpoint -- read the handler, do not assume
   it from the route existing.
5. Confirm no code path logs a full `transaction_intents` row or a full build request/response
   body by default -- grep for the table/struct name next to a logging call. Pool, amounts,
   pubkey, and idempotency key are fine to log; nothing else in that row should reach a log
   line by default.
6. Confirm `wallets.pubkey` is genuinely the only per-user identifier of its kind -- no other
   table anywhere in the schema has a column that could hold, or be repurposed to hold, a
   private key, seed, or passphrase.
7. Confirm the `miniapp` CI job is green on the commit being released (`npm run typecheck`,
   `npm test`, `npm audit --audit-level=high`). This is necessary and nowhere near sufficient --
   it catches type errors, test regressions, and known-vulnerable dependencies, none of which
   is the same question as items 9-10 below.

**Deployment -- the part no job in this repository, or any repository, can check by itself:**

8. **Bundle integrity.** Confirm what is actually served at the Mini App's registered domain
   right now matches a specific, reviewed commit in `miniapp/` -- not "matches what we think we
   deployed." Whoever can alter what is served controls every passphrase typed and every
   transaction signed from that point forward; this is the actual security-critical path in
   this entire design, more so than anything else in this document, and it deserves to be
   treated that way in practice, not just acknowledged here.
9. **Deploy access.** List everyone and everything (CI service accounts included) with the
   ability to change what is served at that domain. Anyone on that list who does not need to be
   there is a standing risk with no corresponding benefit; remove them before release, not
   after an incident.
10. **The independent-verification step runs before every signature, with no bypass.** Confirm,
    by reading `miniapp/src/verify/` rather than trusting a comment or this document, that it
    decodes the raw instruction bytes it is about to sign and checks: every program id against
    a fixed allow-list; every account this wallet signs as owner/sender/payer equals the
    wallet's own pubkey; every source/destination token account is independently re-derived
    rather than trusted as given; the decoded amounts and ranges match what the UI displayed.
    Confirm a deliberately tampered backend response (wrong owner, wrong program id, wrong
    destination, altered amount -- one change at a time) is rejected before a passphrase prompt
    ever appears, for each tamper case separately.
11. **No unlocked-key window survives navigation.** Confirm a decrypted key does not outlive the
    single signing operation it was decrypted for -- background the tab or navigate away
    mid-flow and confirm the next action re-prompts for the passphrase rather than reusing
    anything held in memory.
12. **Export/reveal-phrase friction.** Confirm the recovery-phrase export flow requires the
    passphrase, displays the phrase only transiently, and has no silent copy-to-clipboard or
    auto-upload path -- the same convenience that makes legitimate recovery easy is what a
    phishing clone of the Mini App would try to induce.

**Operational:**

13. Confirm RPC/Geyser credentials in the deployed environment are least-privilege and were
    rotated after this checklist was last run against a prior release, not still the values
    from initial setup.
14. Confirm the operator on call for this release knows what "the backend served a bad
    transaction" looks like in the logs and what to do about it -- there is no kill-switch that
    can stop a signature already given, so the response is entirely "get the bad version
    un-served," and whoever is on call should know that before it is needed, not during.
