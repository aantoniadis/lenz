# Homebrew formula for a tap (e.g. a `homebrew-lenz` repository): `brew install <owner>/lenz/lenz`.
# packaging/bump.sh fills in the version and checksums from a release.
class Lenz < Formula
  desc "Code reader, diff viewer, Git client and merge tool with GUI and terminal frontends"
  homepage "https://github.com/aantoniadis/lenz"
  version "0.0.4"
  license all_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/aantoniadis/lenz/releases/download/v#{version}/lenz-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "5cb66334d6eec730ad0114f58a872af5f57dbd199f5ff6a9265649617fb15f0f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/aantoniadis/lenz/releases/download/v#{version}/lenz-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "5f901b9c7a019cbb5723eb73122f47cc21666026ceead7198361084c99c207f7"
    end
    on_arm do
      url "https://github.com/aantoniadis/lenz/releases/download/v#{version}/lenz-#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "c9b9fd25e7b989f19fbb1c3ec3327ad0e25df819d58190527cbbe9458d0f7c37"
    end
  end

  depends_on "git"

  def install
    executables = %w[lenz lenz-code lenz-code-tui lenz-diff lenz-diff-tui]
    if OS.mac?
      # The bundle gives the Dock the Lenz icon from launch to quit; the launcher finds the apps beside its real file.
      prefix.install "Lenz.app"
      executables.each { |name| bin.install_symlink prefix/"Lenz.app/Contents/MacOS/#{name}" }
    else
      bin.install(*executables)
    end
    doc.install "README.md"
    doc.install "LICENSE-THEMES" if File.exist?("LICENSE-THEMES")
    if OS.linux?
      (share/"applications").install "lenz.desktop", "lenz-diff.desktop"
      # one 1024 px PNG: pixmaps is the size-independent location the icon lookup always searches
      (share/"pixmaps").install "lenz-icon.png" => "lenz.png"
    end
  end

  def caveats
    on_linux do
      <<~EOS
        The GUI loads the system's libxkbcommon-x11, libxcb and Vulkan loader at runtime
        (Debian/Ubuntu: libxkbcommon-x11-0 libxcb1 libvulkan1; Fedora: libxkbcommon-x11 libxcb vulkan-loader).
        Desktop installations have them; a minimal server image may not. lenz --tui and lenz diff --tui need none.
      EOS
    end
  end

  test do
    assert_match "Usage: lenz [file-or-folder]", shell_output("#{bin}/lenz --help")
    assert_match "Usage: lenz --tui", shell_output("#{bin}/lenz --tui --help")
    assert_match "usage: lenz diff [options]", shell_output("#{bin}/lenz diff --bogus 2>&1", 2)
    assert_match "usage: lenz diff --tui", shell_output("#{bin}/lenz diff --tui --bogus 2>&1", 2)
  end
end
