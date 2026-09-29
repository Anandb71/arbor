# Arbor Homebrew formula
# Checksums are written by the release workflow (update-checksums job).
# No tap is published yet; see docs/INSTALL.md for macOS/Linux installs.
class Arbor < Formula
  desc "Graph-native intelligence for codebases — know what breaks before you break it"
  homepage "https://github.com/Anandb71/arbor"
  license "MIT"
  version "3.0.3"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-aarch64.tar.gz"
      sha256 "178139614d0342fe6f6218c32bcff228b9e9a1fe8dd66b8c53b5ece6e2d3314c"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-macos-x86_64.tar.gz"
      sha256 "7280a5d5fe999ca2fa4d0b5d2ee4cd87e176565ed538b33b7d129d503a614921"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-aarch64.tar.gz"
      sha256 "d7423651cf52ae3fce56160066205c6c0f71a87dcb68adb1a1b7818d0a2befdc"
    else
      url "https://github.com/Anandb71/arbor/releases/download/v#{version}/arbor-linux-x86_64.tar.gz"
      sha256 "d047de606bc3e1756f6ff1b0c5b3b8ba3d1d440b0bda08df9171184810f72137"
    end
  end

  def install
    bin.install "arbor"
  end

  test do
    assert_match "arbor", shell_output("#{bin}/arbor --version")
  end
end
