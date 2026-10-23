# Arbor Homebrew formula
# Checksums are written by the release workflow (update-checksums job).
# No tap is published yet; see docs/INSTALL.md for macOS/Linux installs.
class Arbor < Formula
  desc "Graph-native intelligence for codebases — know what breaks before you break it"
  homepage "https://github.com/Anandb71/arbor"
  license "MIT"
  version "3.1.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-aarch64.tar.gz"
      sha256 "3fd56ccf110c00b899cb1e25c5999488b60ef167ab692d433498fdf9a69acc46"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-x86_64.tar.gz"
      sha256 "be849ac61b65b76b80468e6b8c0b1dd6e5d8ed382273d04708d18b15384f90f7"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-aarch64.tar.gz"
      sha256 "a80cc600fe9b981f41a62490315f57f90d0f1078e1ee637893cf337c73b2a1fa"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-x86_64.tar.gz"
      sha256 "f7b95718092f65eba41ff55036eddb361e4a8a32bf521b8e2c7854124b1ba525"
    end
  end

  def install
    bin.install "arbor"
  end

  test do
    assert_match "arbor", shell_output("#{bin}/arbor --version")
  end
end
