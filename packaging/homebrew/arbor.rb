# Arbor Homebrew formula
# Checksums are written by the release workflow (update-checksums job).
# No tap is published yet; see docs/INSTALL.md for macOS/Linux installs.
class Arbor < Formula
  desc "Graph-native intelligence for codebases — know what breaks before you break it"
  homepage "https://github.com/Anandb71/arbor"
  license "MIT"
  version "3.0.4"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-aarch64.tar.gz"
      sha256 "914180b96348b01477b2ceaa97b62d60169a294a08aabbb967ae9dc8a647d798"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-x86_64.tar.gz"
      sha256 "1026fe024abfe140174f14966afa3f2793049ed8693c037c2e0cfab4658848c6"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-aarch64.tar.gz"
      sha256 "08b79bf72ae7b7bc266158ded3604dd1f3c8cbe32c5c38cb086f21923dbf7878"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-x86_64.tar.gz"
      sha256 "67985904e7c96383a2e3665f3be46608dbbf8d05531cce5978b19a904c1174e0"
    end
  end

  def install
    bin.install "arbor"
  end

  test do
    assert_match "arbor", shell_output("#{bin}/arbor --version")
  end
end
