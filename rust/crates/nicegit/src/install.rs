//! Installing a newer NiceGit from inside the app. The download, its checksum check and the
//! platform's own install steps run on a background thread, and the interface shows the stage.
//! Nothing is installed from a file that fails its checksum, and the download link stays the
//! fallback wherever this cannot run.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use nicegit_core::install::{asset_for_method, fresh_temp_dir, release_url, verify_download, InstallMethod};

use crate::app::NiceGitApp;
use crate::tools::Task;
use crate::ui::dialogs::{Dialog, Pending};

/// The downloads an install makes: the release's checksum list and one release file. Tests put
/// stand-ins in their place, so no test reaches the network.
#[derive(Clone)]
pub struct Source {
    pub checksums: ChecksumsFn,
    pub download: DownloadFn,
}

/// Fetches the checksum list of a release, for its version.
pub type ChecksumsFn = Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// Downloads a URL to a path.
pub type DownloadFn = Arc<dyn Fn(&str, &Path) -> Result<(), String> + Send + Sync>;

/// What an install left behind, which decides how the app is started again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    /// The app bundle that was replaced, to open again.
    Bundle(PathBuf),
    /// The AppImage that was replaced, to run again.
    AppImage(PathBuf),
    /// The MSI installer was started. NiceGit closes so the installer can replace its files.
    Installer,
}

/// An install running in the background.
pub struct Install {
    pub version: String,
    /// The stage the installer is in, such as "Downloading…", shown in the notice.
    pub(crate) stage: Arc<Mutex<&'static str>>,
    pub(crate) task: Task<Result<Installed, String>>,
}

#[cfg(test)]
thread_local! {
    /// The install method tests report, instead of the one this build was found to have.
    pub static METHOD_OVERRIDE: std::cell::RefCell<Option<InstallMethod>> = const { std::cell::RefCell::new(None) };
    /// The downloads tests make instead of the real ones.
    pub static SOURCE_OVERRIDE: std::cell::RefCell<Option<Source>> = const { std::cell::RefCell::new(None) };
}

/// How this copy of NiceGit was installed. It is found once, from the running executable.
/// Tests answer Manual unless they inject another method.
pub fn current_method() -> InstallMethod {
    #[cfg(test)]
    {
        METHOD_OVERRIDE.with(|method| method.borrow().clone()).unwrap_or(InstallMethod::Manual)
    }
    #[cfg(not(test))]
    {
        static METHOD: std::sync::OnceLock<InstallMethod> = std::sync::OnceLock::new();
        METHOD.get_or_init(detect_method).clone()
    }
}

/// Whether this copy can replace itself with `version`: the platform has a release file for it,
/// and the install method is one NiceGit can install over.
pub fn offers_install(version: &str) -> bool {
    asset_for_method(&current_method(), version, std::env::consts::ARCH).is_some()
}

/// The note the update notice adds when Homebrew owns the copy, since NiceGit will not replace it.
pub fn manual_update_hint() -> Option<&'static str> {
    matches!(current_method(), InstallMethod::Homebrew).then_some("Update with: brew upgrade --cask nicegit")
}

#[cfg(not(test))]
fn detect_method() -> InstallMethod {
    use nicegit_core::install::install_method;
    let Ok(exe) = std::env::current_exe() else { return InstallMethod::Manual };
    match install_method(&exe, |name| std::env::var(name).ok()) {
        InstallMethod::MacBundle(bundle) => {
            // A Homebrew cask copies the app into /Applications, but leaves its folder in the Caskroom.
            if ["/opt/homebrew/Caskroom/nicegit", "/usr/local/Caskroom/nicegit"].iter().any(|dir| Path::new(dir).is_dir()) {
                InstallMethod::Homebrew
            } else if bundle.parent().is_some_and(dir_writable) {
                InstallMethod::MacBundle(bundle)
            } else {
                InstallMethod::Manual
            }
        }
        InstallMethod::AppImage(path) => {
            if path.parent().is_some_and(dir_writable) {
                InstallMethod::AppImage(path)
            } else {
                InstallMethod::Manual
            }
        }
        other => other,
    }
}

/// Whether a new file can be made in `dir`, checked by making and removing a probe file.
#[cfg(not(test))]
fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".nicegit-write-check-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn current_source() -> Source {
    #[cfg(test)]
    {
        SOURCE_OVERRIDE.with(|source| source.borrow().clone()).unwrap_or_else(refused_source)
    }
    #[cfg(not(test))]
    {
        Source {
            checksums: Arc::new(nicegit_core::install::fetch_checksums),
            download: Arc::new(|url: &str, dest: &Path| nicegit_core::install::download(url, dest)),
        }
    }
}

/// The downloads tests get when they have not injected any: they fail at once.
#[cfg(test)]
fn refused_source() -> Source {
    Source {
        checksums: Arc::new(|_: &str| Err("Downloads are not made in tests.".to_string())),
        download: Arc::new(|_: &str, _: &Path| Err("Downloads are not made in tests.".to_string())),
    }
}

fn set_stage(stage: &Mutex<&'static str>, text: &'static str) {
    if let Ok(mut slot) = stage.lock() {
        *slot = text;
    }
}

/// What an install needs about the release it installs.
struct Release<'a> {
    version: &'a str,
    asset: &'a str,
    sums: &'a str,
    source: &'a Source,
    stage: &'a Mutex<&'static str>,
}

impl Release<'_> {
    /// Downloads the release file to `dest`, then checks it. A file that fails is deleted.
    fn fetch(&self, dest: &Path) -> Result<(), String> {
        set_stage(self.stage, "Downloading…");
        if let Err(error) = (self.source.download)(&release_url(self.version, self.asset), dest) {
            let _ = std::fs::remove_file(dest);
            return Err(format!("The download failed: {error}"));
        }
        set_stage(self.stage, "Checking…");
        verify_download(dest, self.sums, self.asset)
    }
}

/// The whole install, on the background thread.
fn run_install(method: InstallMethod, version: &str, source: &Source, stage: &Mutex<&'static str>) -> Result<Installed, String> {
    let Some(asset) = asset_for_method(&method, version, std::env::consts::ARCH) else {
        return Err("This copy of NiceGit cannot install an update itself.".to_string());
    };
    set_stage(stage, "Downloading…");
    let sums = (source.checksums)(version)?;
    let release = Release { version, asset: &asset, sums: &sums, source, stage };
    match method {
        InstallMethod::MacBundle(bundle) => install_bundle(&bundle, &release),
        InstallMethod::WindowsMsi => install_msi(&release),
        InstallMethod::AppImage(path) => install_appimage(&path, &release),
        InstallMethod::Homebrew | InstallMethod::Manual => Err("This copy of NiceGit cannot install an update itself.".to_string()),
    }
}

/// Replaces the app bundle with the verified disk image's copy.
fn install_bundle(bundle: &Path, release: &Release) -> Result<Installed, String> {
    let dir = fresh_temp_dir().map_err(|error| format!("No temporary folder could be made: {error}"))?;
    let dmg = dir.join(release.asset);
    let result = release.fetch(&dmg).and_then(|()| replace_bundle(bundle, &dmg, &dir.join("mount"), release.stage));
    // Only empty folders are removed here: a disk image that is still mounted keeps its contents.
    let _ = std::fs::remove_file(&dmg);
    let _ = std::fs::remove_dir(dir.join("mount"));
    let _ = std::fs::remove_dir(&dir);
    result.map(|()| Installed::Bundle(bundle.to_path_buf()))
}

/// Copies the checked app out of the disk image beside the current bundle, then swaps it in.
fn replace_bundle(bundle: &Path, dmg: &Path, mount: &Path, stage: &Mutex<&'static str>) -> Result<(), String> {
    let parent = bundle.parent().ok_or("The app bundle has no folder.")?;
    let staged = parent.join(".NiceGit-update.app");
    remove_path(&staged)?;
    // The update must be signed by the same Apple team as the copy running now; an unsigned
    // running copy (a development build) never replaces itself.
    let team = team_identifier(bundle).ok_or("This copy of NiceGit is not signed, so it cannot verify an update. Download it instead.")?;
    let copied = stage_from_disk_image(dmg, mount, &staged, &team, stage);
    if let Err(error) = copied {
        let _ = remove_path(&staged);
        return Err(error);
    }
    set_stage(stage, "Installing…");
    if let Err(error) = swap_bundle(bundle, &staged) {
        let _ = remove_path(&staged);
        return Err(error);
    }
    Ok(())
}

/// Mounts the disk image, checks its app with codesign and Gatekeeper, copies it to `staged`, and
/// always detaches the image again.
fn stage_from_disk_image(dmg: &Path, mount: &Path, staged: &Path, team: &str, stage: &Mutex<&'static str>) -> Result<(), String> {
    std::fs::create_dir(mount).map_err(|error| format!("No mount folder could be made: {error}"))?;
    attach(dmg, mount)?;
    let app = mount.join("NiceGit.app");
    let copied = check_and_copy(&app, staged, team, stage);
    let detached = detach(mount);
    copied.and(detached)
}

/// The Apple team that signed `bundle`, from `codesign`'s description of it. None when the bundle
/// is unsigned or signed ad hoc.
fn team_identifier(bundle: &Path) -> Option<String> {
    let output = Command::new("/usr/bin/codesign").args(["-d", "--verbose=2"]).arg(bundle).output().ok()?;
    // codesign describes the signature on standard error.
    let text = String::from_utf8_lossy(&output.stderr);
    team_from_codesign(&text)
}

/// The `TeamIdentifier=` value in codesign's description, when it names a team: ten letters or
/// digits, as Apple issues them.
fn team_from_codesign(text: &str) -> Option<String> {
    let team = text.lines().find_map(|line| line.strip_prefix("TeamIdentifier="))?.trim();
    (team.len() == 10 && team.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())).then(|| team.to_string())
}

fn check_and_copy(app: &Path, staged: &Path, team: &str, stage: &Mutex<&'static str>) -> Result<(), String> {
    const UNVERIFIED: &str = "The new NiceGit could not be verified, so it was not installed.";
    set_stage(stage, "Checking…");
    // A valid signature is not enough: it must be a Developer ID signature from the same team.
    let requirement = format!("=anchor apple generic and certificate leaf[subject.OU] = \"{team}\"");
    if !run(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict", "-R"]).arg(&requirement).arg(app)) {
        return Err(UNVERIFIED.to_string());
    }
    if !run(Command::new("/usr/sbin/spctl").args(["--assess", "--type", "execute"]).arg(app)) {
        return Err(UNVERIFIED.to_string());
    }
    set_stage(stage, "Installing…");
    if !run(Command::new("/usr/bin/ditto").arg(app).arg(staged)) {
        return Err("The new copy could not be made beside NiceGit.".to_string());
    }
    Ok(())
}

fn attach(dmg: &Path, mount: &Path) -> Result<(), String> {
    let mut command = Command::new("/usr/bin/hdiutil");
    command.args(["attach", "-nobrowse", "-readonly", "-mountpoint"]).arg(mount).arg(dmg);
    if run(&mut command) {
        Ok(())
    } else {
        Err("The disk image could not be opened.".to_string())
    }
}

fn detach(mount: &Path) -> Result<(), String> {
    if run(Command::new("/usr/bin/hdiutil").arg("detach").arg(mount))
        || run(Command::new("/usr/bin/hdiutil").args(["detach", "-force"]).arg(mount))
    {
        Ok(())
    } else {
        Err("The disk image could not be closed.".to_string())
    }
}

/// Runs a command quietly and reports whether it succeeded.
fn run(command: &mut Command) -> bool {
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|status| status.success())
}

/// Moves the current bundle aside, moves the new copy into its place, and deletes the old one. If
/// the new copy cannot be moved in, the old bundle is put back.
fn swap_bundle(bundle: &Path, staged: &Path) -> Result<(), String> {
    let parent = bundle.parent().ok_or("The app bundle has no folder.")?;
    let old = parent.join(".NiceGit-old.app");
    remove_path(&old)?;
    std::fs::rename(bundle, &old).map_err(|error| format!("NiceGit could not be moved aside: {error}"))?;
    if let Err(error) = std::fs::rename(staged, bundle) {
        let _ = std::fs::rename(&old, bundle);
        return Err(format!("The new copy could not be put in place: {error}"));
    }
    // A failure to delete the old copy leaves a hidden folder behind, which does no harm.
    let _ = remove_path(&old);
    Ok(())
}

/// Removes a file or folder, and nothing when it is not there.
fn remove_path(path: &Path) -> Result<(), String> {
    let result = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => Err(error),
    };
    result.map_err(|error| format!("{} could not be removed: {error}", path.display()))
}

/// Starts the MSI installer detached, so it outlives NiceGit.
fn install_msi(release: &Release) -> Result<Installed, String> {
    let dir = fresh_temp_dir().map_err(|error| format!("No temporary folder could be made: {error}"))?;
    let msi = dir.join(release.asset);
    let result = release.fetch(&msi).and_then(|()| {
        set_stage(release.stage, "Installing…");
        // The installer reads the file while it runs, so the folder is left for it.
        Command::new("msiexec")
            .arg("/i")
            .arg(&msi)
            .arg("/passive")
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("The installer did not start: {error}"))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&msi);
        let _ = std::fs::remove_dir(&dir);
    }
    result.map(|()| Installed::Installer)
}

/// Downloads the AppImage beside the current one as `<name>.new`, checks it, and renames it over
/// the current file.
fn install_appimage(path: &Path, release: &Release) -> Result<Installed, String> {
    let file_name = path.file_name().ok_or("The AppImage has no file name.")?;
    let mut staged_name = file_name.to_os_string();
    staged_name.push(".new");
    let staged = path.with_file_name(staged_name);
    release.fetch(&staged)?;
    set_stage(release.stage, "Installing…");
    let placed = make_executable(&staged)
        .and_then(|()| std::fs::rename(&staged, path).map_err(|error| format!("The new AppImage could not be put in place: {error}")));
    if placed.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    placed.map(|()| Installed::AppImage(path.to_path_buf()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|error| format!("The new AppImage could not be made runnable: {error}"))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

impl NiceGitApp {
    /// Opens the confirmation for installing `version`. Nothing is downloaded until it is confirmed.
    pub fn open_install_dialog(&mut self, version: &str) {
        let message = if matches!(current_method(), InstallMethod::WindowsMsi) {
            "NiceGit will close while the installer runs."
        } else {
            "NiceGit will restart."
        };
        self.dialog = Some(Dialog::Confirm {
            title: format!("Install NiceGit {version}?"),
            message: message.to_string(),
            button: "Install".to_string(),
            pending: Pending::InstallUpdate { version: version.to_string() },
        });
    }

    /// Whether an install may start now: nothing is running, no action is busy, and the editor
    /// holds no unsaved text.
    pub fn install_allowed(&self) -> bool {
        self.install.is_none() && self.busy.is_none() && !self.has_unsaved_editor()
    }

    /// Starts installing `version` in the background. Refused while an action runs or the editor
    /// has unsaved text, as any change to the checkout is.
    pub fn start_install(&mut self, version: String) {
        if self.install.is_some() {
            return;
        }
        if self.busy.is_some() {
            self.notify("Wait for the current action to finish before installing NiceGit.", true);
            return;
        }
        if self.has_unsaved_editor() {
            self.notify("Save or close the file you are editing before installing NiceGit.", true);
            return;
        }
        // The version names the download address, so it must be a plain release number.
        if nicegit_core::updates::parse_version(&version).is_none()
            || !version.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            self.notify("The update's version could not be read, so it was not installed. Download it instead.", true);
            return;
        }
        if !offers_install(&version) {
            self.notify("This copy of NiceGit cannot install the update itself. Download it instead.", true);
            return;
        }
        let method = current_method();
        let source = current_source();
        let stage = Arc::new(Mutex::new("Downloading…"));
        let worker_stage = stage.clone();
        let worker_version = version.clone();
        let context = self.worker.context.clone();
        let task = Task::spawn(&context, move || run_install(method, &worker_version, &source, &worker_stage));
        self.install = Some(Install { version, stage, task });
    }

    /// Hands a finished install to the notice: a restart is offered, an error is shown, and the
    /// MSI installer closes NiceGit at once.
    pub fn finish_install(&mut self, ctx: &egui::Context) {
        let Some(install) = self.install.as_mut() else { return };
        let Some(result) = install.task.get().cloned() else { return };
        let version = install.version.clone();
        self.install = None;
        match result {
            Ok(Installed::Installer) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Ok(installed) => self.installed_update = Some((version, installed)),
            Err(reason) => self.notify(reason, true),
        }
    }

    /// The stage of the install running now, for the notice, with the version it installs.
    pub fn install_progress(&self) -> Option<(String, &'static str)> {
        let install = self.install.as_ref()?;
        let stage = install.stage.lock().map(|stage| *stage).unwrap_or("Installing…");
        Some((install.version.clone(), stage))
    }

    /// Starts the installed copy and closes this one. Refused while the editor has unsaved text,
    /// since closing NiceGit would lose it.
    pub fn restart_after_install(&mut self, ctx: &egui::Context) {
        if self.has_unsaved_editor() {
            self.notify("Save or close the file you are editing before restarting NiceGit.", true);
            return;
        }
        let Some((_, installed)) = self.installed_update.clone() else { return };
        let started = match &installed {
            Installed::Bundle(bundle) => Command::new("/usr/bin/open").arg("-n").arg(bundle).spawn(),
            Installed::AppImage(path) => Command::new(path).spawn(),
            Installed::Installer => return,
        };
        match started {
            Ok(_) => {
                self.installed_update = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(error) => self.notify(format!("NiceGit could not restart: {error}"), true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_replacement_leaves_the_old_bundle_when_the_new_one_cannot_be_moved_in() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("NiceGit.app");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        std::fs::write(bundle.join("Contents/marker"), "old").unwrap();
        // The staged copy is missing, so the swap cannot move it into place.
        let staged = dir.path().join(".NiceGit-update.app");
        let error = swap_bundle(&bundle, &staged).unwrap_err();
        assert!(error.contains("could not be put in place"), "{error}");
        assert_eq!(std::fs::read_to_string(bundle.join("Contents/marker")).unwrap(), "old", "the original is restored");
        assert!(!dir.path().join(".NiceGit-old.app").exists());
    }

    #[test]
    fn a_bundle_replacement_swaps_in_the_new_copy_and_deletes_the_old() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("NiceGit.app");
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        std::fs::write(bundle.join("Contents/marker"), "old").unwrap();
        let staged = dir.path().join(".NiceGit-update.app");
        std::fs::create_dir_all(staged.join("Contents")).unwrap();
        std::fs::write(staged.join("Contents/marker"), "new").unwrap();
        swap_bundle(&bundle, &staged).unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("Contents/marker")).unwrap(), "new");
        assert!(!staged.exists());
        assert!(!dir.path().join(".NiceGit-old.app").exists(), "the old copy is deleted");
    }

    #[test]
    fn a_leftover_from_an_earlier_attempt_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("NiceGit.app");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("marker"), "old").unwrap();
        let staged = dir.path().join(".NiceGit-update.app");
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("marker"), "new").unwrap();
        let stale = dir.path().join(".NiceGit-old.app");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("junk"), "x").unwrap();
        swap_bundle(&bundle, &staged).unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("marker")).unwrap(), "new");
        assert!(!stale.exists());
    }

    #[test]
    fn the_signing_team_is_read_from_codesign() {
        let signed = "Executable=/Applications/NiceGit.app/Contents/MacOS/nicegit\nAuthority=Developer ID Application: Someone (AB12CD34EF)\nTeamIdentifier=AB12CD34EF\n";
        assert_eq!(team_from_codesign(signed).as_deref(), Some("AB12CD34EF"));
        assert_eq!(team_from_codesign("Signature=adhoc\nTeamIdentifier=not set\n"), None);
        assert_eq!(team_from_codesign("code object is not signed at all"), None);
        assert_eq!(team_from_codesign("TeamIdentifier=ab12\"; rm\n"), None);
    }

    #[test]
    fn removing_a_missing_path_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(remove_path(&dir.path().join("nothing-here")), Ok(()));
    }
}
