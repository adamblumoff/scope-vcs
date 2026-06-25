pub fn install_script(base_url: &str) -> String {
    let base_url = base_url.trim_end_matches('/');
    format!(
        r#"#!/bin/sh
set -eu

base_url="{base_url}"
install_dir="${{SCOPE_INSTALL_DIR:-$HOME/.local/bin}}"
target="$(uname -s)-$(uname -m)"

case "$target" in
  Linux-x86_64|Linux-amd64)
    artifact="scope-x86_64-unknown-linux-gnu"
    ;;
  *)
    echo "scope installer does not have a binary for $target yet." >&2
    exit 1
    ;;
esac

mkdir -p "$install_dir"
curl -fsSL "$base_url/downloads/$artifact" -o "$install_dir/scope"
chmod +x "$install_dir/scope"
echo "scope installed to $install_dir/scope"
"#,
    )
}

#[cfg(test)]
mod tests {
    use super::install_script;

    #[test]
    fn install_script_uses_service_base_url() {
        let script = install_script("https://scope-cli.up.railway.app/");

        assert!(script.contains(r#"base_url="https://scope-cli.up.railway.app""#));
        assert!(script.contains(r#"$base_url/downloads/$artifact"#));
        assert!(script.contains("SCOPE_INSTALL_DIR"));
    }
}
