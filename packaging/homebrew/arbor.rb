# Arbor Homebrew formula
# Install: brew install getArbor-dev/tap/arbor
class Arbor < Formula
  desc "Graph-native intelligence for codebases — know what breaks before you break it"
  homepage "https://github.com/getArbor-dev/arbor"
  license "MIT"
  version "3.0.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/getArbor-dev/arbor/releases/download/v#{version}/arbor-macos-aarch64.tar.gz"
      sha256 "4781d29120d7e2ca78b2794ffea0ed23c57144a26de958d9c1cfb0f2ed1138fb"
    else
      url "https://github.com/getArbor-dev/arbor/releases/download/v#{version}/arbor-macos-x86_64.tar.gz"
      sha256 "add8a34885d5aac0c57de0d1ab9123847f060618c1e44cd2c8d1e1eeef934bc4"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/getArbor-dev/arbor/releases/download/v#{version}/arbor-linux-aarch64.tar.gz"
      sha256 "f9e2b06999494900328e731af56a8098bdce3b75d8b934a7b761552c127da044"
    else
      url "https://github.com/getArbor-dev/arbor/releases/download/v#{version}/arbor-linux-x86_64.tar.gz"
      sha256 "2304b7dd9e1291df971689b60d219cce3a95079cba7916e5f267d20f295109aa"
    end
  end

  def install
    bin.install "arbor"
  end

  test do
    assert_match "arbor", shell_output("#{bin}/arbor --version")
  end
end
