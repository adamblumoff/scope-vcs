use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output_path = std::env::var_os("SCOPE_API_TS_EXPORT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("../web/src/api/types.generated.ts"));
    let schema_output_path = std::env::var_os("SCOPE_API_SCHEMA_EXPORT_PATH")
        .map(PathBuf::from)
        .expect("SCOPE_API_SCHEMA_EXPORT_PATH must point to a temporary schema file");
    api::export_api_contract(&output_path, &schema_output_path);
}
