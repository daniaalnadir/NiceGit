//! Installing a newer NiceGit from inside the app. This module picks the release file that belongs
//! to this build, fetches it with `curl`, and checks it against the release's `SHA256SUMS.txt`.
//! Nothing is installed unless that check passes. The platform steps that use a verified file live
//! in the app crate.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::github::find_program;
use crate::runner::search_path;
use crate::updates::failure_reason;

/// The folder GitHub serves release files from, as `<base>/v<version>/<file>`.
pub const RELEASE_DOWNLOADS: &str = "https://github.com/daniaalnadir/NiceGit/releases/download";

/// The file in each release that lists the SHA-256 of every asset.
pub const CHECKSUMS_FILE: &str = "SHA256SUMS.txt";

/// The message when a download's hash is not the one the release published.
pub const CHECKSUM_MISMATCH: &str = "The download did not match its published checksum, so it was not installed.";

/// How this copy of NiceGit was installed, which decides whether the app can replace itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallMethod {
    /// A `NiceGit.app` bundle, at the path given.
    MacBundle(PathBuf),
    /// The MSI installer, which put NiceGit under `%ProgramFiles%\NiceGit`.
    WindowsMsi,
    /// An AppImage, whose file is at the path given (from `APPIMAGE`).
    AppImage(PathBuf),
    /// A Homebrew cask. Homebrew owns the copy, so NiceGit does not replace it.
    Homebrew,
    /// Anything else: portable copies, packages from a distribution, development builds.
    Manual,
}

/// The release asset for a platform, as `os` and `arch` name it (`std::env::consts` values), or
/// `None` when no release file exists for that platform. macOS has one universal disk image.
pub fn asset_name(version: &str, os: &str, arch: &str) -> Option<String> {
    match (os, arch) {
        ("macos", _) => Some(format!("NiceGit-{version}-macos-universal.dmg")),
        ("windows", "x86_64") => Some(format!("NiceGit-{version}-windows-x64.msi")),
        ("windows", "aarch64") => Some(format!("NiceGit-{version}-windows-arm64.msi")),
        ("linux", "x86_64" | "aarch64") => Some(format!("NiceGit-{version}-linux-{arch}.AppImage")),
        _ => None,
    }
}

/// The asset an install method replaces with: the one for the platform that method belongs to.
pub fn asset_for_method(method: &InstallMethod, version: &str, arch: &str) -> Option<String> {
    let os = match method {
        InstallMethod::MacBundle(_) => "macos",
        InstallMethod::WindowsMsi => "windows",
        InstallMethod::AppImage(_) => "linux",
        InstallMethod::Homebrew | InstallMethod::Manual => return None,
    };
    asset_name(version, os, arch)
}

/// The address of a file in the release for `version`, such as `2.1.0`.
pub fn release_url(version: &str, file: &str) -> String {
    format!("{RELEASE_DOWNLOADS}/v{version}/{file}")
}

/// The SHA-256 that `sums_text` lists for `filename`, in lower case. Lines read as `<hex>  <name>`,
/// as `sha256sum` writes them, and also with the `*` binary marker. Only an exact name matches.
/// `None` when no line names the file, when the hash is not 64 hex digits, or when two lines name
/// it with different hashes.
pub fn expected_checksum(sums_text: &str, filename: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for line in sums_text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some((hash, rest)) = line.split_once(' ') else { continue };
        let name = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('*'));
        if name != Some(filename) {
            continue;
        }
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let hash = hash.to_ascii_lowercase();
        match &found {
            Some(existing) if *existing != hash => return None,
            _ => found = Some(hash),
        }
    }
    found
}

/// The SHA-256 of a file, in lower case hex. Reads in pieces, so large files are not held in memory.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Checks a downloaded `asset` (named `filename` in the release) against `sums_text`. When the
/// check fails for any reason, the file is deleted, so nothing unverified can be installed later.
pub fn verify_download(asset: &Path, sums_text: &str, filename: &str) -> Result<(), String> {
    let result = compare_checksum(asset, sums_text, filename);
    if result.is_err() {
        let _ = std::fs::remove_file(asset);
    }
    result
}

fn compare_checksum(asset: &Path, sums_text: &str, filename: &str) -> Result<(), String> {
    let expected = expected_checksum(sums_text, filename)
        .ok_or_else(|| format!("{CHECKSUMS_FILE} has no checksum for {filename}, so it was not installed."))?;
    let actual = sha256_file(asset).map_err(|error| format!("The download could not be read: {error}"))?;
    if actual.eq_ignore_ascii_case(&expected) {
        Ok(())
    } else {
        Err(CHECKSUM_MISMATCH.to_string())
    }
}

/// Downloads `url` to `destination` with `curl`, over HTTPS only. Blocks until curl finishes, so
/// it runs on a background thread.
pub fn download(url: &str, destination: &Path) -> Result<(), String> {
    let curl = find_program("curl").ok_or_else(|| "curl is not installed.".to_string())?;
    let mut command = Command::new(curl);
    command
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            "600",
            "-o",
        ])
        .arg(destination)
        .arg(url)
        .env("PATH", search_path())
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this, the download flashes a console window on Windows.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().map_err(|error| format!("curl did not start: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failure_reason(&String::from_utf8_lossy(&output.stderr)))
    }
}

/// The text of the release's `SHA256SUMS.txt` for `version`, fetched into a temporary folder that
/// is removed again.
pub fn fetch_checksums(version: &str) -> Result<String, String> {
    let dir = fresh_temp_dir().map_err(|error| format!("No temporary folder could be made: {error}"))?;
    let path = dir.join(CHECKSUMS_FILE);
    let result = download(&release_url(version, CHECKSUMS_FILE), &path)
        .and_then(|()| std::fs::read_to_string(&path).map_err(|error| format!("The checksums could not be read: {error}")));
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// A new, empty folder under the system's temporary folder, with a name no other run uses.
pub fn fresh_temp_dir() -> io::Result<PathBuf> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("nicegit-update-{}-{nanos}", std::process::id()));
    std::fs::create_dir(&dir)?;
    Ok(dir)
}

/// How this copy was installed, from the path of its executable and the environment variables
/// that matter (`env` returns a variable's value, or `None`). It does not check that any folder is
/// writable; the app does that before offering to install.
pub fn install_method(exe: &Path, env: impl Fn(&str) -> Option<String>) -> InstallMethod {
    let text = exe.to_string_lossy().replace('\\', "/");
    if text.starts_with("/opt/homebrew/") || text.starts_with("/usr/local/Caskroom/") {
        return InstallMethod::Homebrew;
    }
    if let Some(bundle) = app_bundle(exe) {
        return InstallMethod::MacBundle(bundle);
    }
    if let Some(appimage) = env("APPIMAGE").filter(|path| !path.is_empty()) {
        return InstallMethod::AppImage(PathBuf::from(appimage));
    }
    if env("ProgramFiles").is_some_and(|folder| under_program_files(&text, &folder)) {
        return InstallMethod::WindowsMsi;
    }
    InstallMethod::Manual
}

/// The `NiceGit.app` bundle whose executable is `exe`, when `exe` is `.../NiceGit.app/Contents/MacOS/nicegit`.
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    if exe.file_name()? != "nicegit" {
        return None;
    }
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && bundle.file_name()? == "NiceGit.app";
    is_bundle.then(|| bundle.to_path_buf())
}

/// Whether the Windows path `exe` (with `/` or `\` separators) is inside `<program_files>\NiceGit`.
/// Windows paths compare without case.
fn under_program_files(exe: &str, program_files: &str) -> bool {
    let normal = |text: &str| text.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
    let folder = normal(program_files);
    if folder.is_empty() {
        return false;
    }
    normal(exe).starts_with(&format!("{folder}/nicegit/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn sums(lines: &[&str]) -> String {
        lines.join("\n")
    }

    fn none_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn each_platform_gets_its_own_release_file() {
        assert_eq!(asset_name("2.1.0", "macos", "x86_64").as_deref(), Some("NiceGit-2.1.0-macos-universal.dmg"));
        assert_eq!(asset_name("2.1.0", "macos", "aarch64").as_deref(), Some("NiceGit-2.1.0-macos-universal.dmg"));
        assert_eq!(asset_name("2.1.0", "windows", "x86_64").as_deref(), Some("NiceGit-2.1.0-windows-x64.msi"));
        assert_eq!(asset_name("2.1.0", "windows", "aarch64").as_deref(), Some("NiceGit-2.1.0-windows-arm64.msi"));
        assert_eq!(asset_name("2.1.0", "linux", "x86_64").as_deref(), Some("NiceGit-2.1.0-linux-x86_64.AppImage"));
        assert_eq!(asset_name("2.1.0", "linux", "aarch64").as_deref(), Some("NiceGit-2.1.0-linux-aarch64.AppImage"));
    }

    #[test]
    fn platforms_without_a_release_file_get_none() {
        assert_eq!(asset_name("2.1.0", "linux", "riscv64"), None);
        assert_eq!(asset_name("2.1.0", "windows", "x86"), None);
        assert_eq!(asset_name("2.1.0", "freebsd", "x86_64"), None);
    }

    #[test]
    fn an_install_method_picks_the_asset_for_its_own_platform() {
        let bundle = InstallMethod::MacBundle(PathBuf::from("/Applications/NiceGit.app"));
        assert_eq!(asset_for_method(&bundle, "2.1.0", "aarch64").as_deref(), Some("NiceGit-2.1.0-macos-universal.dmg"));
        assert_eq!(asset_for_method(&InstallMethod::WindowsMsi, "2.1.0", "x86_64").as_deref(), Some("NiceGit-2.1.0-windows-x64.msi"));
        let appimage = InstallMethod::AppImage(PathBuf::from("/home/me/NiceGit.AppImage"));
        assert_eq!(asset_for_method(&appimage, "2.1.0", "x86_64").as_deref(), Some("NiceGit-2.1.0-linux-x86_64.AppImage"));
        assert_eq!(asset_for_method(&InstallMethod::Manual, "2.1.0", "x86_64"), None);
        assert_eq!(asset_for_method(&InstallMethod::Homebrew, "2.1.0", "x86_64"), None);
    }

    #[test]
    fn release_files_come_from_the_tagged_release_folder() {
        assert_eq!(
            release_url("2.1.0", "SHA256SUMS.txt"),
            "https://github.com/daniaalnadir/NiceGit/releases/download/v2.1.0/SHA256SUMS.txt"
        );
    }

    #[test]
    fn a_checksum_is_found_by_exact_file_name() {
        let text = sums(&[
            &format!("{HASH_ABC}  NiceGit-2.1.0-macos-universal.dmg"),
            &format!("{HASH_ABC}  NiceGit-2.1.0-macos-universal.dmg.sig"),
        ]);
        assert_eq!(expected_checksum(&text, "NiceGit-2.1.0-macos-universal.dmg").as_deref(), Some(HASH_ABC));
        assert_eq!(expected_checksum(&text, "NiceGit-2.1.0-macos-universal"), None, "a prefix does not match");
        assert_eq!(expected_checksum(&text, "nicegit-2.1.0-macos-universal.dmg"), None, "names are case-sensitive");
    }

    #[test]
    fn a_missing_checksum_reads_as_none() {
        let text = sums(&[&format!("{HASH_ABC}  NiceGit-2.1.0-windows-x64.msi")]);
        assert_eq!(expected_checksum(&text, "NiceGit-2.1.0-linux-x86_64.AppImage"), None);
        assert_eq!(expected_checksum("", "NiceGit-2.1.0-windows-x64.msi"), None);
    }

    #[test]
    fn the_binary_marker_and_windows_line_endings_are_tolerated() {
        let text = format!("{HASH_ABC} *NiceGit-2.1.0-linux-x86_64.AppImage\r\n");
        assert_eq!(expected_checksum(&text, "NiceGit-2.1.0-linux-x86_64.AppImage").as_deref(), Some(HASH_ABC));
    }

    #[test]
    fn uppercase_hex_is_read_in_lower_case() {
        let text = format!("{}  NiceGit-2.1.0-linux-x86_64.AppImage", HASH_ABC.to_uppercase());
        assert_eq!(expected_checksum(&text, "NiceGit-2.1.0-linux-x86_64.AppImage").as_deref(), Some(HASH_ABC));
    }

    #[test]
    fn malformed_or_conflicting_entries_read_as_none() {
        let short = format!("{}  NiceGit.dmg", &HASH_ABC[..60]);
        assert_eq!(expected_checksum(&short, "NiceGit.dmg"), None, "a hash must be 64 hex digits");
        let not_hex = format!("{}  NiceGit.dmg", "z".repeat(64));
        assert_eq!(expected_checksum(&not_hex, "NiceGit.dmg"), None);
        let other = "0".repeat(64);
        let conflict = sums(&[&format!("{HASH_ABC}  NiceGit.dmg"), &format!("{other}  NiceGit.dmg")]);
        assert_eq!(expected_checksum(&conflict, "NiceGit.dmg"), None, "two different hashes for one file are not trusted");
        let repeated = sums(&[&format!("{HASH_ABC}  NiceGit.dmg"), &format!("{HASH_ABC}  NiceGit.dmg")]);
        assert_eq!(expected_checksum(&repeated, "NiceGit.dmg").as_deref(), Some(HASH_ABC));
    }

    #[test]
    fn sha256_is_computed_over_the_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(sha256_file(&path).unwrap(), HASH_ABC);

        let big = dir.path().join("big.bin");
        std::fs::write(&big, vec![7u8; 200_000]).unwrap();
        let again = sha256_file(&big).unwrap();
        assert_eq!(again.len(), 64);
        assert!(sha256_file(&dir.path().join("missing.bin")).is_err());
    }

    #[test]
    fn a_download_with_the_published_checksum_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("NiceGit-2.1.0-linux-x86_64.AppImage");
        std::fs::write(&path, b"abc").unwrap();
        let text = format!("{}  NiceGit-2.1.0-linux-x86_64.AppImage\n", HASH_ABC.to_uppercase());
        assert_eq!(verify_download(&path, &text, "NiceGit-2.1.0-linux-x86_64.AppImage"), Ok(()));
        assert!(path.exists());
    }

    #[test]
    fn a_download_that_does_not_match_is_deleted_and_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("NiceGit-2.1.0-macos-universal.dmg");
        std::fs::write(&path, b"tampered").unwrap();
        let text = format!("{HASH_ABC}  NiceGit-2.1.0-macos-universal.dmg\n");
        assert_eq!(verify_download(&path, &text, "NiceGit-2.1.0-macos-universal.dmg"), Err(CHECKSUM_MISMATCH.to_string()));
        assert!(!path.exists(), "a mismatched file is deleted");
    }

    #[test]
    fn a_download_without_a_listed_checksum_is_deleted_and_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("NiceGit-2.1.0-windows-x64.msi");
        std::fs::write(&path, b"abc").unwrap();
        let error = verify_download(&path, "", "NiceGit-2.1.0-windows-x64.msi").unwrap_err();
        assert!(error.contains("has no checksum for NiceGit-2.1.0-windows-x64.msi"), "{error}");
        assert!(!path.exists());
    }

    #[test]
    fn a_download_that_cannot_be_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.dmg");
        let text = format!("{HASH_ABC}  gone.dmg\n");
        assert!(verify_download(&path, &text, "gone.dmg").unwrap_err().contains("could not be read"));
    }

    #[test]
    fn a_mac_app_bundle_is_found_from_its_executable() {
        let exe = Path::new("/Applications/NiceGit.app/Contents/MacOS/nicegit");
        assert_eq!(install_method(exe, none_env), InstallMethod::MacBundle(PathBuf::from("/Applications/NiceGit.app")));
        let user = Path::new("/Users/me/Apps/NiceGit.app/Contents/MacOS/nicegit");
        assert_eq!(install_method(user, none_env), InstallMethod::MacBundle(PathBuf::from("/Users/me/Apps/NiceGit.app")));
    }

    #[test]
    fn an_executable_outside_a_bundle_layout_is_not_a_bundle() {
        for path in [
            "/Applications/NiceGit.app/nicegit",
            "/Applications/Other.app/Contents/MacOS/nicegit",
            "/Applications/NiceGit.app/Contents/MacOS/nicegit-dev",
            "/Applications/NiceGit.app/Contents/Resources/nicegit",
            "/Users/me/target/debug/nicegit",
        ] {
            assert_eq!(install_method(Path::new(path), none_env), InstallMethod::Manual, "{path}");
        }
    }

    #[test]
    fn homebrew_installs_are_recognised_before_anything_else() {
        let caskroom = Path::new("/usr/local/Caskroom/nicegit/2.0.0/NiceGit.app/Contents/MacOS/nicegit");
        assert_eq!(install_method(caskroom, none_env), InstallMethod::Homebrew);
        let arm = Path::new("/opt/homebrew/bin/nicegit");
        assert_eq!(install_method(arm, none_env), InstallMethod::Homebrew);
    }

    #[test]
    fn an_appimage_is_found_from_the_appimage_variable() {
        let env = |name: &str| (name == "APPIMAGE").then(|| "/home/me/Apps/NiceGit.AppImage".to_string());
        let exe = Path::new("/tmp/.mount_NiceGiXYZ/usr/bin/nicegit");
        assert_eq!(install_method(exe, env), InstallMethod::AppImage(PathBuf::from("/home/me/Apps/NiceGit.AppImage")));
        let empty = |name: &str| (name == "APPIMAGE").then(String::new);
        assert_eq!(install_method(exe, empty), InstallMethod::Manual, "an empty variable is not an AppImage");
    }

    #[test]
    fn the_msi_install_is_recognised_under_program_files() {
        let env = |name: &str| (name == "ProgramFiles").then(|| r"C:\Program Files".to_string());
        let installed = Path::new(r"C:\Program Files\NiceGit\nicegit.exe");
        assert_eq!(install_method(installed, env), InstallMethod::WindowsMsi);
        let other_case = Path::new(r"c:\program files\nicegit\nicegit.exe");
        assert_eq!(install_method(other_case, env), InstallMethod::WindowsMsi, "Windows paths compare without case");
    }

    #[test]
    fn portable_and_other_windows_copies_are_manual() {
        let env = |name: &str| (name == "ProgramFiles").then(|| r"C:\Program Files".to_string());
        for path in [
            r"C:\Users\me\Downloads\NiceGit\nicegit.exe",
            r"C:\Users\me\AppData\Local\Microsoft\WinGet\Packages\NiceGit\nicegit.exe",
            r"C:\Program Files\NiceGitTools\nicegit.exe",
            r"D:\NiceGit\nicegit.exe",
        ] {
            assert_eq!(install_method(Path::new(path), env), InstallMethod::Manual, "{path}");
        }
        assert_eq!(install_method(Path::new(r"C:\Program Files\NiceGit\nicegit.exe"), none_env), InstallMethod::Manual, "no ProgramFiles");
    }

    #[test]
    fn distribution_packages_and_development_builds_are_manual() {
        assert_eq!(install_method(Path::new("/usr/bin/nicegit"), none_env), InstallMethod::Manual, ".deb or .rpm");
        assert_eq!(install_method(Path::new("/Users/me/PROJECTS/nicegit/rust/target/debug/nicegit"), none_env), InstallMethod::Manual);
    }
}
