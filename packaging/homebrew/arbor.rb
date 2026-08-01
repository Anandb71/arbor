# Arbor Homebrew formula
# Install: brew install Anandb71/tap/arbor
class Arbor < Formula
  desc "Graph-native intelligence for codebases — know what breaks before you break it"
  homepage "https://github.com/Anandb71/arbor"
  license "MIT"
  version "2.6.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-aarch64.tar.gz"
      sha256 "733d2b9e8871be2b0c757cefe5a2d7cefc7e4375a7f15f2796b6c9680d940634"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-x86_64.tar.gz"
      sha256 "9de570cf05d5cf7a50d1c6e9c9a0081dfa22a3fe5b8a05c3b9058aab9bfdc032"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-aarch64.tar.gz"
      sha256 "7ef2123047198f6678d8ef714e477edd1cbd53846bba6ed2c09090950722d4f3"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-x86_64.tar.gz"
      sha256 "9a84fc6654c8569c0a2c954c1093bdcbf5afc942b704f24dc6408c115f8222f8"
    end
  end

  def install
    bin.install "arbor"
  end

  test do
    assert_match "arbor", shell_output("#{bin}/arbor --version")
  end
end
