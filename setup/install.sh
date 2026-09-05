#!/usr/bin/env bash
set -euo pipefail

# Resolve and validate version
version=$(sed -nE 's/^version = "([^"]+)"$/\1/p' "$GITHUB_ACTION_PATH/../crates/wepub/Cargo.toml")
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "::error::Invalid version: \"$version\""
  exit 1
fi

# Map runner platform to release artifact parameters
case "$RUNNER_OS-$RUNNER_ARCH" in
  Linux-X64)   target=x86_64-unknown-linux-gnu  archive_ext=.tar.xz binary_ext=     ;;
  Linux-ARM64) target=aarch64-unknown-linux-gnu archive_ext=.tar.xz binary_ext=     ;;
  macOS-X64)   target=x86_64-apple-darwin       archive_ext=.tar.xz binary_ext=     ;;
  macOS-ARM64) target=aarch64-apple-darwin      archive_ext=.tar.xz binary_ext=     ;;
  Windows-X64) target=x86_64-pc-windows-msvc    archive_ext=.zip    binary_ext=.exe ;;
  *)
    echo "::error::Unsupported platform: $RUNNER_OS $RUNNER_ARCH"
    exit 1
    ;;
esac

# Check tool cache
tool_dir="$RUNNER_TOOL_CACHE/wepub/$version/$RUNNER_ARCH"
if [[ -f "$tool_dir/wepub$binary_ext" ]]; then
  echo "wepub $version is already cached"
  echo "$tool_dir" >> "$GITHUB_PATH"
  echo "version=$version" >> "$GITHUB_OUTPUT"
  exit 0
fi

# Download archive
archive="wepub-$target$archive_ext"
base_url="https://github.com/iorate/wepub/releases/download/wepub-v$version"
echo "Downloading $base_url/$archive"
curl -fsSL "$base_url/$archive" -o "$RUNNER_TEMP/$archive"

# Verify build provenance
gh attestation verify "$RUNNER_TEMP/$archive" \
  --repo iorate/wepub \
  --signer-workflow iorate/wepub/.github/workflows/wepub-release.yml
echo "Verified the build provenance of $archive"

# Extract binary
extract_dir="$RUNNER_TEMP/wepub-$version"
mkdir -p "$extract_dir"
if [[ "$archive_ext" == ".tar.xz" ]]; then
  # Tarballs nest the binary under a wepub-$target/ directory.
  tar -xJf "$RUNNER_TEMP/$archive" -C "$extract_dir" --strip-components=1 "wepub-$target/wepub$binary_ext"
else
  # The zip places the binary at the archive root.
  unzip -q "$RUNNER_TEMP/$archive" "wepub$binary_ext" -d "$extract_dir"
fi

# Verify extracted binary version
version_output="$("$extract_dir/wepub$binary_ext" --version)"
if [[ "$version_output" != "wepub $version" ]]; then
  echo "::error::Unexpected version output: \"$version_output\" (expected \"wepub $version\")"
  exit 1
fi

# Move binary into tool cache
mkdir -p "$tool_dir"
mv "$extract_dir/wepub$binary_ext" "$tool_dir/"

echo "$tool_dir" >> "$GITHUB_PATH"
echo "version=$version" >> "$GITHUB_OUTPUT"
echo "Installed wepub $version"
