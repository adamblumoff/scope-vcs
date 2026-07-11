use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output_path = std::env::var_os("SCOPE_API_TS_EXPORT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("../web/src/api/types.generated.ts"));
    api::export_api_types(&output_path);
}
