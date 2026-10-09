# Arbor Homebrew formula
# Checksums are written by the release workflow (update-checksums job).
# No tap is published yet; see docs/INSTALL.md for macOS/Linux installs.
class Arbor < Formula
  desc "Graph-native intelligence for codebases — know what breaks before you break it"
  homepage "https://github.com/Anandb71/arbor"
  license "MIT"
  version "3.0.5"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-aarch64.tar.gz"
      sha256 "5d7e70c92c17310c5a5d0c881b817ee5d3f334845172df1f4efc253882d4f50b"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-x86_64.tar.gz"
      sha256 "b04f943de9f65961f3ab252b15147a91e8e8d66251d9d21c0944fbdf0bd6c8b6"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-aarch64.tar.gz"
      sha256 "07e59418436dc1fe3e9a9a40ffd027019429a47b08f2fbeda8e8d01986f3dec4"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-x86_64.tar.gz"
      sha256 "45289939f1a79f5c885e65f329f0d146c12cf8aa89d5c6a48ac8aff6715ef670"
    end
  end

  def install
    bin.install "arbor"
  end

  test do
    assert_match "arbor", shell_output("#{bin}/arbor --version")
  end
end
