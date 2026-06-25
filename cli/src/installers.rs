use crate::distribution::{DistributionManifest, DistributionTarget};

pub fn posix_install_script(base_url: &str, manifest: &DistributionManifest) -> String {
    let base_url = base_url.trim_end_matches('/');
    let cases = manifest
        .targets()
        .iter()
        .filter_map(posix_case_arm)
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"#!/bin/sh
set -eu

base_url="{base_url}"
install_dir="${{SCOPE_INSTALL_DIR:-$HOME/.local/bin}}"
target="$(uname -s)-$(uname -m)"

case "$target" in
{cases}
  *)
    echo "scope installer does not have a binary for $target yet." >&2
    exit 1
    ;;
esac

mkdir -p "$install_dir"
tmp_file="$(mktemp)"
checksum_file="$(mktemp)"
trap 'rm -f "$tmp_file" "$checksum_file"' EXIT

curl -fsSL "$base_url/downloads/$artifact" -o "$tmp_file"
curl -fsSL "$base_url/downloads/$artifact.sha256" -o "$checksum_file"

expected="$(awk '{{print $1}}' "$checksum_file")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp_file" | awk '{{print $1}}')"
else
  actual="$(shasum -a 256 "$tmp_file" | awk '{{print $1}}')"
fi

if [ "$expected" != "$actual" ]; then
  echo "scope checksum verification failed for $artifact." >&2
  exit 1
fi

mv "$tmp_file" "$install_dir/scope"
chmod +x "$install_dir/scope"
echo "scope installed to $install_dir/scope"
"#,
    )
}

pub fn windows_install_script(base_url: &str, manifest: &DistributionManifest) -> String {
    let base_url = base_url.trim_end_matches('/');
    let cases = manifest
        .targets()
        .iter()
        .filter_map(windows_case_arm)
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"$ErrorActionPreference = "Stop"

$baseUrl = "{base_url}"
$installDir = if ($env:SCOPE_INSTALL_DIR) {{ $env:SCOPE_INSTALL_DIR }} else {{ Join-Path $HOME ".local\bin" }}
$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()

switch ($arch) {{
{cases}
  default {{
    throw "scope installer does not have a Windows binary for $arch yet."
  }}
}}

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
$tmpFile = New-TemporaryFile
$checksumFile = New-TemporaryFile
$tmpPath = $tmpFile.FullName
$checksumPath = $checksumFile.FullName

try {{
  Invoke-WebRequest -Uri "$baseUrl/downloads/$artifact" -OutFile $tmpPath
  Invoke-WebRequest -Uri "$baseUrl/downloads/$artifact.sha256" -OutFile $checksumPath

  $expected = ((Get-Content -LiteralPath $checksumPath | Select-Object -First 1) -split "\s+")[0].ToLowerInvariant()
  $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $tmpPath).Hash.ToLowerInvariant()
  if ($expected -ne $actual) {{
    throw "scope checksum verification failed for $artifact."
  }}

  $destination = Join-Path $installDir "scope.exe"
  Move-Item -LiteralPath $tmpPath -Destination $destination -Force
  Write-Output "scope installed to $destination"
}} finally {{
  Remove-Item -LiteralPath $tmpPath -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $checksumPath -Force -ErrorAction SilentlyContinue
}}
"#,
    )
}

fn posix_case_arm(target: &DistributionTarget) -> Option<String> {
    let pattern = match (target.os.as_str(), target.arch.as_str()) {
        ("linux", "x64") => "  Linux-x86_64|Linux-amd64)",
        ("linux", "arm64") => "  Linux-aarch64|Linux-arm64)",
        ("macos", "x64") => "  Darwin-x86_64|Darwin-amd64)",
        ("macos", "arm64") => "  Darwin-arm64|Darwin-aarch64)",
        _ => return None,
    };

    Some(format!(
        r#"{pattern}
    artifact="{artifact}"
    ;;
"#,
        artifact = target.artifact
    ))
}

fn windows_case_arm(target: &DistributionTarget) -> Option<String> {
    match (target.os.as_str(), target.arch.as_str()) {
        ("windows", "x64") => Some(format!(
            r#"  "X64" {{
    $artifact = "{artifact}"
  }}
"#,
            artifact = target.artifact
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{posix_install_script, windows_install_script};
    use crate::distribution::DistributionManifest;

    #[test]
    fn posix_installer_supports_linux_and_macos_targets() {
        let script = posix_install_script(
            "https://scope-cli.up.railway.app/",
            DistributionManifest::bundled(),
        );

        assert!(script.contains(r#"base_url="https://scope-cli.up.railway.app""#));
        assert!(script.contains("Linux-x86_64|Linux-amd64"));
        assert!(script.contains("Linux-aarch64|Linux-arm64"));
        assert!(script.contains("Darwin-x86_64|Darwin-amd64"));
        assert!(script.contains("Darwin-arm64|Darwin-aarch64"));
        assert!(script.contains("$base_url/downloads/$artifact.sha256"));
    }

    #[test]
    fn windows_installer_supports_windows_x64() {
        let script = windows_install_script(
            "https://scope-cli.up.railway.app/",
            DistributionManifest::bundled(),
        );

        assert!(script.contains(r#"$baseUrl = "https://scope-cli.up.railway.app""#));
        assert!(script.contains(r#""X64" {"#));
        assert!(script.contains("scope-x86_64-pc-windows-msvc.exe"));
        assert!(script.contains("scope.exe"));
    }
}
