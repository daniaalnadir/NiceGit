fn main() {
    // Give the Windows executable its icon and version details.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../assets/NiceGit.ico");
        resource.set("ProductName", "NiceGit");
        resource.set("FileDescription", "NiceGit");
        resource.compile().expect("failed to embed the Windows icon");
    }
}
