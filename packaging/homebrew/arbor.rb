# Arbor Homebrew formula
# Checksums are written by the release workflow (update-checksums job).
# No tap is published yet; see docs/INSTALL.md for macOS/Linux installs.
class Arbor < Formula
  desc "Graph-native intelligence for codebases — know what breaks before you break it"
  homepage "https://github.com/Anandb71/arbor"
  license "MIT"
  version "3.1.1"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-aarch64.tar.gz"
      sha256 "84964333e38fae570e5b51a825256f43b57bd218a18c204535e0434185a69dec"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-x86_64.tar.gz"
      sha256 "f90e624dd54e4b150415cca8e0a28bfcd2de63a2a4d54a99d8eda3c34f90cd20"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-aarch64.tar.gz"
      sha256 "4ee667adfb4e80628f1c35ebacad2ef2522c07dd00796ce56acffc7c59e19311"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-x86_64.tar.gz"
      sha256 "e276b078e8adb7e29c5c84234771d0e7dda2169b74aa76c23089f4ffd7718691"
    end
  end

  def install
    bin.install "arbor"
  end

  test do
    assert_match "arbor", shell_output("#{bin}/arbor --version")
  end
end
