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
target="$(uname -s)-$(uname -m)"

case "$target" in
{cases}
  *)
    echo "scope installer does not have a binary for $target yet." >&2
    exit 1
    ;;
esac

path_contains() {{
  case ":${{PATH:-}}:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}}

can_sudo() {{
  [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1
}}

select_install_dir() {{
  if [ -n "${{SCOPE_INSTALL_DIR:-}}" ]; then
    install_dir="$SCOPE_INSTALL_DIR"
    if path_contains "$install_dir" && {{ [ ! -d "$install_dir" ] || [ ! -w "$install_dir" ]; }} && can_sudo; then
      install_with_sudo=1
    fi
    return
  fi

  for candidate in "$HOME/.local/bin" "$HOME/bin" "$HOME/.cargo/bin" "/opt/homebrew/bin" "/usr/local/bin"; do
    if [ -d "$candidate" ] && [ -w "$candidate" ] && path_contains "$candidate"; then
      install_dir="$candidate"
      return
    fi
  done

  old_ifs="$IFS"
  IFS=:
  for candidate in ${{PATH:-}}; do
    IFS="$old_ifs"
    case "$candidate" in
      /*)
        if [ -d "$candidate" ] && [ -w "$candidate" ]; then
          install_dir="$candidate"
          return
        fi
        ;;
    esac
    IFS=:
  done
  IFS="$old_ifs"

  if can_sudo; then
    for candidate in "/usr/local/bin" "/opt/homebrew/bin"; do
      if [ -d "$candidate" ] && path_contains "$candidate"; then
        install_dir="$candidate"
        install_with_sudo=1
        return
      fi
    done

    old_ifs="$IFS"
    IFS=:
    for candidate in ${{PATH:-}}; do
      IFS="$old_ifs"
      case "$candidate" in
        /*)
          if [ -d "$candidate" ]; then
            install_dir="$candidate"
            install_with_sudo=1
            return
          fi
          ;;
      esac
      IFS=:
    done
    IFS="$old_ifs"
  fi
}}

install_dir=""
install_with_sudo=0
select_install_dir
if [ -z "$install_dir" ]; then
  echo "scope installer could not find a writable directory on PATH." >&2
  echo "Add a user bin directory to PATH or rerun with SCOPE_INSTALL_DIR set to a writable PATH directory." >&2
  exit 1
fi

if ! path_contains "$install_dir"; then
  echo "scope install directory is not on PATH: $install_dir" >&2
  echo "Set SCOPE_INSTALL_DIR to a writable directory already on PATH and rerun." >&2
  exit 1
fi

if [ "$install_with_sudo" = 1 ]; then
  sudo mkdir -p "$install_dir"
else
  mkdir -p "$install_dir"
fi

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

if [ "$install_with_sudo" = 1 ]; then
  sudo mv "$tmp_file" "$install_dir/scope"
  sudo chmod 755 "$install_dir/scope"
else
  mv "$tmp_file" "$install_dir/scope"
  chmod 755 "$install_dir/scope"
fi
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

function ConvertTo-ComparablePath([string] $path) {{
  try {{
    return [System.IO.Path]::GetFullPath($path).TrimEnd([char[]]@('\', '/')).ToLowerInvariant()
  }} catch {{
    return $path.TrimEnd([char[]]@('\', '/')).ToLowerInvariant()
  }}
}}

function Test-PathListContains([string] $pathList, [string] $directory) {{
  if ([string]::IsNullOrWhiteSpace($pathList)) {{
    return $false
  }}

  $needle = ConvertTo-ComparablePath $directory
  foreach ($entry in $pathList -split [System.Text.RegularExpressions.Regex]::Escape([System.IO.Path]::PathSeparator)) {{
    if ([string]::IsNullOrWhiteSpace($entry)) {{
      continue
    }}

    if ((ConvertTo-ComparablePath $entry) -eq $needle) {{
      return $true
    }}
  }}

  return $false
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
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
  $processPath = [Environment]::GetEnvironmentVariable("Path", "Process")
  if (
    -not (Test-PathListContains $userPath $installDir) -and
    -not (Test-PathListContains $machinePath $installDir) -and
    -not (Test-PathListContains $processPath $installDir)
  ) {{
    $separator = [System.IO.Path]::PathSeparator
    $nextUserPath = if ([string]::IsNullOrWhiteSpace($userPath)) {{
      $installDir
    }} else {{
      "$userPath$separator$installDir"
    }}
    [Environment]::SetEnvironmentVariable("Path", $nextUserPath, "User")
  }}

  if (-not (Test-PathListContains $env:Path $installDir)) {{
    $separator = [System.IO.Path]::PathSeparator
    $env:Path = if ([string]::IsNullOrWhiteSpace($env:Path)) {{
      $installDir
    }} else {{
      "$env:Path$separator$installDir"
    }}
  }}

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
        ("windows", "arm64") => Some(format!(
            r#"  "Arm64" {{
    $artifact = "{artifact}"
  }}
"#,
            artifact = target.artifact
        )),
        _ => None,
    }
}
