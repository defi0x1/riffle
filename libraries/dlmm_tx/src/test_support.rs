//! Shared by every instruction builder's unit tests. Not compiled outside `#[cfg(test)]`.

use solana_sdk::pubkey::Pubkey;
use std::fs;

pub fn pubkey(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

/// Loads the vendored copy of the public IDL and returns the named instruction's JSON entry, so
/// account-list and discriminator assertions read the source of truth directly instead of a
/// second hand-transcribed copy that could drift from it.
pub fn idl_instruction(name: &str) -> serde_json::Value {
    let raw = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dlmm.json"
    ))
    .expect("reading vendored IDL fixture");
    let idl: serde_json::Value = serde_json::from_str(&raw).expect("parsing vendored IDL fixture");
    idl["instructions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ix| ix["name"] == name)
        .unwrap_or_else(|| panic!("instruction {name} missing from vendored IDL"))
        .clone()
}

/// Asserts a built instruction's discriminator and *named* account list (order, signer flags,
/// writable flags) match the vendored IDL's entry for `idl_name`. Every builder's tests call
/// this so the check can't silently rot into a copy of what the builder already does.
///
/// Only checks the IDL's declared accounts, not any trailing remaining accounts (bin arrays,
/// transfer-hook accounts) a v2 instruction appends after them -- those aren't part of the
/// static account list the IDL describes, and each builder's own tests check them separately.
pub fn assert_matches_idl(ix: &solana_sdk::instruction::Instruction, idl_name: &str) {
    let idl_ix = idl_instruction(idl_name);

    let idl_disc: Vec<u8> = idl_ix["discriminator"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(
        &ix.data[..8],
        idl_disc.as_slice(),
        "discriminator mismatch for {idl_name}"
    );

    let idl_accounts = idl_ix["accounts"].as_array().unwrap();
    assert!(
        ix.accounts.len() >= idl_accounts.len(),
        "built instruction for {idl_name} has fewer accounts ({}) than the IDL's named list ({})",
        ix.accounts.len(),
        idl_accounts.len()
    );
    for (built, idl) in ix.accounts.iter().zip(idl_accounts.iter()) {
        let expect_signer = idl["signer"].as_bool().unwrap_or(false);
        let expect_writable = idl["writable"].as_bool().unwrap_or(false);
        assert_eq!(
            built.is_signer, expect_signer,
            "signer flag for {idl_name}.{}",
            idl["name"]
        );
        assert_eq!(
            built.is_writable, expect_writable,
            "writable flag for {idl_name}.{}",
            idl["name"]
        );
    }
}
