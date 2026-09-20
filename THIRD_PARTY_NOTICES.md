# Third-Party Notices

NiceGit's own source is covered by the root MIT LICENSE.

## SwiftTerm 1.20.0

- Source: https://github.com/migueldeicaza/SwiftTerm
- License: MIT, including the upstream copyright notices for Miguel de Icaza,
  xterm.js authors, SourceLair, and Christopher Jeffrey.
- Used for the embedded terminal. Its complete license is copied verbatim from
  the resolved dependency into `NiceGit.app/Contents/Resources/SwiftTerm-LICENSE`
  by the packaging script.

## Swift Argument Parser

- Source: https://github.com/apple/swift-argument-parser
- License: Apache 2.0 with Swift Runtime Library Exception.
- Transitive SwiftTerm build-tool dependency; the resolved version is recorded
  in Package.resolved. It is not a NiceGit app runtime dependency. Its license is
  available in the dependency checkout as LICENSE.txt.

Git and GitHub CLI are external user-installed tools, not bundled with NiceGit.
Apple system frameworks are supplied by macOS. No GitKraken artwork is bundled.
# GitHub Logo Artwork

The bundled GitHub Invertocat images are from https://brand.github.com/GitHub_Logos.zip.
GitHub's logo is a trademark of GitHub, Inc. and remains subject to GitHub's brand
guidelines at https://brand.github.com/foundations/logo, not NiceGit's MIT license.
It identifies GitHub-hosted remotes and does not imply endorsement.
