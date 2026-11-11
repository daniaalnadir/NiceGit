cask "nicegit" do
  version "2.0.0"
  sha256 "4446d579611f376692d8ceabae2579166028c96e2a4292fd2432894a7792497f"

  url "https://github.com/daniaalnadir/NiceGit/releases/download/v#{version}/NiceGit-#{version}-macos-universal.dmg"
  name "NiceGit"
  desc "Fast, free and open-source Git client"
  homepage "https://daniaalnadir.github.io/NiceGit/"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on :macos

  app "NiceGit.app"

  zap trash: [
    "~/Library/Application Support/nicegit",
    "~/Library/Saved Application State/app.nicegit.desktop.savedState",
  ]
end
