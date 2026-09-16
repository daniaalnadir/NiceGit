# Publishing NiceGit

## Source Release Checklist

1. Review README, LICENSE, and third-party notices. Publish as a preview, not as a
   stable or notarized release. The current app version is 0.1.0, build 1.
2. Run `swift test --build-system native` and `bash scripts/build-app.sh release`.
   Smoke-test the packaged app with a disposable repository on the intended OS.
3. Review `git status --short` and `git diff --cached` before committing. Keep
   `.build`, `.swiftpm`, `dist`, credentials, signing keys, and personal files out
   of Git. Package.resolved belongs in the repository.
4. Create an empty GitHub repository under the intended account, without adding
   a second README or license. Choose visibility deliberately. Add its actual
   URL as origin, commit the reviewed source, and push the intended branch.
5. Wait for the Build and test workflow to pass on GitHub. Local success does not
   prove the hosted runner or a different Xcode version works.
6. Enable private vulnerability reporting and review branch protection/settings.
   Link to this project's own repository; do not imply GitKraken affiliation.

The project repository is https://github.com/daniaalnadir/NiceGit. Commits, tags,
pushes, and release publication are deliberate maintainer actions, not part of
the build. Contributors should push to their own fork unless given write access.

CI selects Xcode 26.6 from GitHub's
[macOS 26 image](https://github.com/actions/runner-images/blob/main/images/macos/macos-26-arm64-Readme.md).
Runner images change over time; update that selection deliberately if GitHub
retires it. The workflow has read-only repository permissions and publishes no
release assets automatically.

## Downloadable App Releases

Source publication is ready to be reviewed independently of binary distribution.
The local package is ad-hoc signed and built for the host architecture, not a
universal macOS installer. Do not advertise Intel support without testing it.

Before offering a normal public app download, configure Developer ID signing,
hardened runtime, notarization, and stapling using a maintainer-owned Apple
Developer account. Keep certificates and credentials out of this repository.
Test the resulting downloaded archive on a clean machine with Gatekeeper enabled.
This repository does not yet automate that signing/notarization pipeline.

Include the app's resource licenses in every binary distribution. Bump both
version fields in `packaging/Info.plist`, document changes, and only tag the exact
commit that passed CI and release smoke tests. Never upload `.build` or a private
working repository as a release asset.
