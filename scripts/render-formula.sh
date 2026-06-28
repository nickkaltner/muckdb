#!/usr/bin/env bash
# Render the Homebrew formula for a release to stdout.
#
# Usage: render-formula.sh <version> <dist-dir>
#
# <dist-dir> must contain the release tarballs named
#   muckdb-<version>-<target>.tar.gz
# for these targets:
#   aarch64-apple-darwin, x86_64-unknown-linux-gnu
set -euo pipefail

VERSION="${1:?usage: render-formula.sh <version> <dist-dir>}"
DIST="${2:?usage: render-formula.sh <version> <dist-dir>}"
REPO="${REPO:-nickkaltner/muckdb}"
BASE="https://github.com/${REPO}/releases/download/v${VERSION}"

sha() {
  local target="$1"
  local file="${DIST}/muckdb-${VERSION}-${target}.tar.gz"
  sha256sum "$file" | cut -d' ' -f1
}

ARM_MAC="$(sha aarch64-apple-darwin)"
LINUX="$(sha x86_64-unknown-linux-gnu)"

cat <<EOF
class Muckdb < Formula
  desc "Live web view and history for your duckdb databases"
  homepage "https://github.com/${REPO}"
  version "${VERSION}"
  license "MIT"

  depends_on "duckdb"

  on_macos do
    on_arm do
      url "${BASE}/muckdb-${VERSION}-aarch64-apple-darwin.tar.gz"
      sha256 "${ARM_MAC}"
    end
  end

  on_linux do
    on_intel do
      url "${BASE}/muckdb-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "${LINUX}"
    end
  end

  def install
    bin.install "muckdb"
  end

  def caveats
    <<~EOS
      muckdb ships a Claude Code skill that teaches coding agents how to drive it.
      To install it into your skills directory, run:
        muckdb skill install
      It is written to ~/.claude/skills/muckdb/SKILL.md (--force to update).
      Remove it again with:
        muckdb skill uninstall
    EOS
  end

  test do
    assert_match "v", shell_output("#{bin}/muckdb --version")
  end
end
EOF
