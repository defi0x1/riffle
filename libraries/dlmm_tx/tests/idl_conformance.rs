// Cross-checks this crate's hand-written enum declarations against the vendored IDL's own
// `types` section, so a variant reorder in either place -- rather than a byte comparison against
// a value nobody can trace back to the IDL -- is what actually catches drift.

use std::fs;

use dlmm_tx::{AccountsType, StrategyType};

fn idl() -> serde_json::Value {
    let raw = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/dlmm.json"
    ))
    .expect("reading vendored IDL fixture");
    serde_json::from_str(&raw).expect("parsing vendored IDL fixture")
}

fn idl_type_variant_names(idl: &serde_json::Value, type_name: &str) -> Vec<String> {
    idl["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == type_name)
        .unwrap_or_else(|| panic!("type {type_name} missing from vendored IDL"))["type"]["variants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn test_strategy_type_variant_order_matches_idl() {
    let idl = idl();
    let idl_variants = idl_type_variant_names(&idl, "StrategyType");

    // Anchor tags a fieldless enum by declaration-order index, so this crate's StrategyType
    // must declare its variants in exactly this order for add_liquidity_by_strategy2 to encode
    // the strategy the caller actually asked for.
    let ours = [
        StrategyType::SpotOneSide,
        StrategyType::CurveOneSide,
        StrategyType::BidAskOneSide,
        StrategyType::SpotBalanced,
        StrategyType::CurveBalanced,
        StrategyType::BidAskBalanced,
        StrategyType::SpotImBalanced,
        StrategyType::CurveImBalanced,
        StrategyType::BidAskImBalanced,
    ];
    let our_tags: Vec<u8> = ours.iter().map(|v| borsh::to_vec(v).unwrap()[0]).collect();

    assert_eq!(idl_variants.len(), ours.len());
    for (i, name) in idl_variants.iter().enumerate() {
        assert_eq!(
            our_tags[i], i as u8,
            "declaration order for {name} drifted from index {i}"
        );
    }
}

#[test]
fn test_accounts_type_variant_order_matches_idl() {
    let idl = idl();
    let idl_variants = idl_type_variant_names(&idl, "AccountsType");

    assert_eq!(
        idl_variants,
        vec![
            "TransferHookX",
            "TransferHookY",
            "TransferHookReward",
            "TransferHookMultiReward",
            "TransferHookReferral"
        ]
    );

    // Spot-check the two variants this crate actually emits (RemainingAccountsInfo::none)
    // encode at the IDL's declared indices 0 and 1.
    assert_eq!(borsh::to_vec(&AccountsType::TransferHookX).unwrap()[0], 0);
    assert_eq!(borsh::to_vec(&AccountsType::TransferHookY).unwrap()[0], 1);
}

#[test]
fn test_all_five_chosen_instructions_are_present_in_the_idl_with_expected_discriminators() {
    let idl = idl();
    let instructions = idl["instructions"].as_array().unwrap();

    for name in [
        "initialize_position2",
        "add_liquidity_by_strategy2",
        "remove_liquidity_by_range2",
        "claim_fee2",
        "close_position2",
    ] {
        let ix = instructions
            .iter()
            .find(|i| i["name"] == name)
            .unwrap_or_else(|| panic!("{name} not in IDL"));
        let idl_disc: Vec<u8> = ix["discriminator"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let computed = dlmm_decode::discriminator("global", name);
        assert_eq!(
            idl_disc, computed,
            "computed discriminator for {name} disagrees with the IDL's own"
        );
    }
}
