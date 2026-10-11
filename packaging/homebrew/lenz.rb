# Homebrew formula for a tap (e.g. a `homebrew-lenz` repository): `brew install <owner>/lenz/lenz`.
# packaging/bump.sh fills in the version and checksums from a release.
class Lenz < Formula
  desc "IntelliJ-grade side-by-side diff viewer, git client and merge tool"
  homepage "https://github.com/aantoniadis/lenz"
  version "0.0.0"
  license all_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/aantoniadis/lenz/releases/download/v#{version}/lenz-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "MISSING-aarch64-apple-darwin"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/aantoniadis/lenz/releases/download/v#{version}/lenz-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "MISSING-x86_64-unknown-linux-gnu"
    end
    on_arm do
      url "https://github.com/aantoniadis/lenz/releases/download/v#{version}/lenz-#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "MISSING-aarch64-unknown-linux-gnu"
    end
  end

  depends_on "git"

  def install
    if OS.mac?
      # The bundle gives the Dock the Lenz icon from launch to quit; `lenz` on PATH runs it.
      prefix.install "Lenz.app"
      bin.install_symlink prefix/"Lenz.app/Contents/MacOS/lenz"
      bin.install "lenz-tui"
    else
      bin.install "lenz", "lenz-tui"
    end
    doc.install "README.md"
    if OS.linux?
      (share/"applications").install "lenz.desktop"
      (share/"icons/hicolor/1024x1024/apps").install "lenz-icon.png" => "lenz.png"
    end
  end

  test do
    assert_match "usage: lenz", shell_output("#{bin}/lenz --bogus 2>&1", 2)
    assert_match "usage: lenz-tui", shell_output("#{bin}/lenz-tui --bogus 2>&1", 2)
  end
end
