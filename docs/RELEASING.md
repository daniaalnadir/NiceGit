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

## Cross-platform Releases (Rust)

Releases are made from `main` by `.github/workflows/release.yml`.

1. Update `version` in `rust/Cargo.toml` and add a `CHANGELOG.md` entry, and merge that to
   `main`.
2. Optionally, do a dry run: Actions › Release › Run workflow, on `main`. It builds, signs,
   notarizes, and checks everything, without creating a release.
3. Tag the merged commit, for example `git tag v1.0.0 && git push origin v1.0.0`.
4. When the run finishes, open the draft release, check its notes and files, and publish it.

The tag's run first checks that the tag points to a commit on `main` and matches the version
in `rust/Cargo.toml`. It then builds:

- `NiceGit-<version>-macos-universal.dmg` (Apple Silicon and Intel), built by
  `rust/scripts/package-macos.sh` and checked by `rust/scripts/verify-macos.sh`;
- `NiceGit-<version>-windows-x64.zip`;
- `NiceGit-<version>-linux-x86_64.tar.gz` and `NiceGit-<version>-linux-amd64.deb`;
- `SHA256SUMS.txt`, the checksums of those files;

and creates a draft release with them. A plain version such as `v1.0.0` is a full release, and
GitHub marks it Latest when published; a version with a suffix such as `v1.1.0-rc.1` is a
pre-release. Running the workflow again for the same tag replaces a draft's files. A
published release is never changed: bump the version and tag again instead.

### Signing and notarizing the macOS app

The Apple secrets live in the repository's `release` environment (Settings › Environments),
which admits only `main` and `v*` tags; only the macOS job uses it. Without them the app is
ad-hoc signed and macOS asks people to approve it in System Settings › Privacy & Security
the first time.

| Secret | Contents |
| --- | --- |
| `DEVELOPER_ID_P12` | Base64 of the exported Developer ID Application certificate and private key (`.p12`) |
| `DEVELOPER_ID_P12_PASSWORD` | The password chosen when exporting the `.p12` |

Notarization signs in to Apple with one of these sets:

| Apple ID | Contents |
| --- | --- |
| `APPLE_ID` | The developer account's Apple ID (email) |
| `APPLE_PASSWORD` | An app-specific password from account.apple.com › Sign-In and Security |
| `APPLE_TEAM_ID` | The Team ID from developer.apple.com › Account › Membership details |

| App Store Connect API key | Contents |
| --- | --- |
| `NOTARY_KEY_P8` | Base64 of the API key (`AuthKey_XXXX.p8`) |
| `NOTARY_KEY_ID` | That key's Key ID |
| `NOTARY_ISSUER_ID` | The Issuer ID shown above the keys list in App Store Connect |

Encode files with `base64 -i file | pbcopy`. With the certificate the app is signed with the
hardened runtime and a secure timestamp; with a notarization set too, it is notarized and
the ticket stapled to the DMG. When signing secrets are present, `verify-macos.sh` fails the
run unless the app is signed with Developer ID by `APPLE_TEAM_ID`, Gatekeeper accepts it as
notarized, and the DMG carries its ticket. Without them it warns, and the run's summary says
the build is unsigned. Windows and Linux builds are unsigned.
