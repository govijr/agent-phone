#!/bin/sh
# Installs the latest agent-phone release.
#
#   curl -fsSL https://raw.githubusercontent.com/govijr/agent-phone/main/install.sh | sh
#
# Settings (environment variables):
#   AGENT_PHONE_VERSION      a release tag such as v0.2.0 (default: the latest)
#   AGENT_PHONE_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
set -eu

repo="govijr/agent-phone"
dir="${AGENT_PHONE_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
  echo "agent-phone install: $*" >&2
  exit 1
}

[ "$(uname -s)" = "Darwin" ] || fail "agent-phone runs on macOS only"
[ "$(uname -m)" = "arm64" ] || fail "agent-phone needs an Apple silicon Mac"
command -v curl >/dev/null || fail "curl is missing"
command -v shasum >/dev/null || fail "shasum is missing"

version="${AGENT_PHONE_VERSION:-}"
if [ -z "$version" ]; then
  version=$(curl -fsSL -H "User-Agent: agent-phone-install" "https://api.github.com/repos/$repo/releases/latest" 2>/dev/null |
    sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1) || true
  [ -n "$version" ] || fail "couldn't find the latest release at github.com/$repo/releases"
fi

name="agent-phone-$version-aarch64-apple-darwin.tar.gz"
base="${AGENT_PHONE_BASE_URL:-https://github.com/$repo/releases/download/$version}"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading agent-phone $version"
curl -fsSL "$base/$name" -o "$tmp/$name" || fail "couldn't download $base/$name"
curl -fsSL "$base/$name.sha256" -o "$tmp/$name.sha256" || fail "couldn't download the checksum"

expected=$(cut -d ' ' -f 1 "$tmp/$name.sha256")
actual=$(shasum -a 256 "$tmp/$name" | cut -d ' ' -f 1)
[ -n "$expected" ] && [ "$expected" = "$actual" ] || fail "checksum mismatch; nothing was installed"

tar -xzf "$tmp/$name" -C "$tmp"
"$tmp/agent-phone" --version >/dev/null 2>&1 || fail "the downloaded binary doesn't run"

mkdir -p "$dir"
# Copy, then rename, so an existing install is never left half-written.
cp "$tmp/agent-phone" "$dir/agent-phone.new"
chmod 755 "$dir/agent-phone.new"
mv -f "$dir/agent-phone.new" "$dir/agent-phone"
# A running daemon from the old version restarts on its own, but stop it now to be tidy.
"$dir/agent-phone" daemon stop >/dev/null 2>&1 || true

echo "Installed $("$dir/agent-phone" --version) to $dir/agent-phone"
case ":$PATH:" in
  *":$dir:"*) ;;
  *)
    echo
    echo "$dir is not on your PATH. Add it with:"
    echo "  echo 'export PATH=\"$dir:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
    ;;
esac
echo
echo "Next: agent-phone doctor"
