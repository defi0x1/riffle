#!/bin/sh
# CI guard for the keyless-backend property: the backend builds unsigned transactions and
# never sees, stores, transmits or logs key material -- device custody is the whole point of
# the V2 design. Fails the build if backend Rust source gains the ability to hold or handle
# secret key material.
#
# Two term classes, checked separately because they need different case sensitivity:
#
# IDENT_TERMS -- Rust identifiers whose mere presence means the code can sign or construct a
# signing key: solana_sdk's Keypair and Signer trait, ed25519-dalek's SigningKey/SecretKey.
# Matched case-sensitively (word-bounded) so this does not fire on the existing, legitimate
# lowercase `signer` swap-attribution column (libraries/storage) or on prose like "the
# position keypair is generated client-side" (libraries/dlmm_tx) -- both real, existing,
# harmless uses that happen to share a word with the banned type names.
#
# FIELD_TERMS -- the vocabulary of secret material itself -- mnemonic/BIP39 (and its sibling
# derivation crates), passphrase, seed phrase, private/secret key naming, and the one
# construction method (`from_base58_string`) that turns a bare string into a Keypair without
# the word "Keypair" appearing on the same line. Matched case-insensitively: unlike Signer/
# Keypair, there is no legitimate lowercase use of these words anywhere in backend code, so
# there is nothing to carve out.
#
# NOTE: this script's own doc comments above necessarily name what they forbid. That is fine
# -- this file is a .sh script, outside the .rs/Cargo.toml scope FILES below actually reads,
# so it is never scanned. Do not "fix" this by vaguing up the comments.
set -eu

IDENT_TERMS='\bKeypair\b|\bSigner\b|\bSigningKey\b|\bSecretKey\b'
FIELD_TERMS='mnemonic|bip-?39|bip-?32|slip-?10|hd.?wallet|coins-bip39|ed25519-hd-key|passphrase|seed[_-]?phrase|private[_-]?key|secret[_-]?key|from_base58_string'

# Files that legitimately need to name the above -- test fixtures exercising a real signer
# against a local validator, say. Add an entry only for a specific file, with a comment
# explaining what it's for and why it cannot leak into non-test code; do not add a directory,
# and do not widen this to make a real finding disappear.
#
# libraries/dlmm_tx/tests/onchain_validation.rs: an integration test, gated on a reachable
# validator/RPC endpoint and never part of the normal build, that generates a disposable,
# throwaway keypair entirely inside the test process to sign a couple of transactions sent to
# a local validator it also just funded via airdrop -- proving the real program accepts what
# this crate's builders produce. That keypair is created, used, and discarded within one test
# run; it never derives from a mnemonic, never persists, and never touches any account this
# repo's own users control. This is the client-side signing a real wallet does on the user's
# device (see docs/security.md) reproduced inside a test process so the test can act as that
# device, not a new capability added to the backend itself.
ALLOWLIST='
libraries/dlmm_tx/tests/onchain_validation.rs
'

SELF='scripts/keyless-guard.sh'

cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    FILES=$(git ls-files -- '*.rs' 'Cargo.toml' 'bin/*/Cargo.toml' 'libraries/*/Cargo.toml' 'tests/Cargo.toml' | grep -v -F "$SELF" || true)
else
    FILES=$(find . -type d \( -name .git -o -name target \) -prune -o \
        -type f -name '*.rs' -print -o -type f -name 'Cargo.toml' -print \
        | sed 's#^\./##' | grep -v -F "$SELF" || true)
fi

found=0
for f in $FILES; do
    [ -f "$f" ] || continue
    skip=0
    for a in $ALLOWLIST; do
        [ "$f" = "$a" ] && skip=1
    done
    [ "$skip" -eq 1 ] && continue

    if grep -nE "$IDENT_TERMS" "$f" >/dev/null 2>&1; then
        grep -nE "$IDENT_TERMS" "$f" | while IFS=: read -r lineno rest; do
            echo "keyless-guard: signing-capable identifier in $f:$lineno: $rest"
        done
        found=1
    fi
    if grep -niE "$FIELD_TERMS" "$f" >/dev/null 2>&1; then
        grep -niE "$FIELD_TERMS" "$f" | while IFS=: read -r lineno rest; do
            echo "keyless-guard: key-material term in $f:$lineno: $rest"
        done
        found=1
    fi
done

if [ "$found" -ne 0 ]; then
    echo "keyless-guard: backend source or manifest names key-signing material; the backend" >&2
    echo "keyless-guard: must never hold, derive, or handle a private key -- see docs/security.md" >&2
    exit 1
fi

exit 0
