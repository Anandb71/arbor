# Arbor Homebrew formula
# Install: brew install Anandb71/tap/arbor
class Arbor < Formula
  desc "Graph-native intelligence for codebases — know what breaks before you break it"
  homepage "https://github.com/Anandb71/arbor"
  license "MIT"
  version "2.5.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-aarch64.tar.gz"
      sha256 "faac85e2b922dbbd1ecdf0afccba10e32ed0a16a64c92383a419fd89e55ecda1"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-x86_64.tar.gz"
      sha256 "d3cdfe9d3f998c0a9641eb9d6d93c8a54afdf0f8b71f85ba75b9befd3db606e4"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-aarch64.tar.gz"
      sha256 "895223f8930cd5571d3568b22db22c7ecec38e63b1e6fa4d5bb29b77cdfb7b51"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-x86_64.tar.gz"
      sha256 "715dddb5a17fef4a04f2a2cbc7e449ad08164652d704ff034a86523eb68ab941"
    end
  end

  def install
    bin.install "arbor"
  end

  test do
    assert_match "arbor", shell_output("#{bin}/arbor --version")
  end
end
