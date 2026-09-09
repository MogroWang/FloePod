use super::*;
use schemars::{generate::SchemaSettings, JsonSchema};

fn input_schema<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(
        SchemaSettings::draft2020_12()
            .into_generator()
            .into_root_schema_for::<T>(),
    )
    .unwrap()
}
fn output_schema<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(
        SchemaSettings::draft2020_12()
            .for_serialize()
            .into_generator()
            .into_root_schema_for::<T>(),
    )
    .unwrap()
}
include!(concat!(env!("OUT_DIR"), "/command-contract.rs"));

#[test]
fn ipc_schema_matches_checked_in_contract() {
    let contract =
        serde_json::json!({ "commands": command_schemas(), "events": crate::events::schemas() });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../contracts/ipc.json");
    let actual = serde_json::to_string_pretty(&contract).unwrap() + "\n";
    if std::env::var_os("FLOEPOD_UPDATE_CONTRACT").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
    }
    let expected = std::fs::read_to_string(path).expect(
        "运行 FLOEPOD_UPDATE_CONTRACT=1 cargo test ipc_schema_matches_checked_in_contract 生成契约",
    );
    assert_eq!(
        actual,
        expected.replace("\r\n", "\n"),
        "IPC 契约改变：检查兼容性并重新生成 Rust schema 和 TypeScript"
    );
}
