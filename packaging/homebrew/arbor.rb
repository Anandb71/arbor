# Arbor Homebrew formula
# Install: brew install Anandb71/tap/arbor
class Arbor < Formula
  desc "Graph-native intelligence for codebases — know what breaks before you break it"
  homepage "https://github.com/Anandb71/arbor"
  license "MIT"
  version "2.4.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-aarch64.tar.gz"
      sha256 "1931b468fbae44f475f647ffade99f1560ca7052aa6ce5a837e69c1288fef549"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-x86_64.tar.gz"
      sha256 "f9ff2e5ff6e3a644273a562749e7aef1ab3fbd436c4ebd5622fd85984879d8b2"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-aarch64.tar.gz"
      sha256 "ab9777faff0a484e7d97d967c4692c9e20ea7d121d9b3693bef5d30f521f0129"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-x86_64.tar.gz"
      sha256 "4151b8c287079cfd5b20e83d8cc4d9cc29c349d5e9cd53876ecef19e295a91a8"
    end
  end

  def install
    bin.install "arbor"
  end

  test do
    assert_match "arbor", shell_output("#{bin}/arbor --version")
  end
end
