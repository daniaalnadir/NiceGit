use std::fmt::Write as _;
use std::path::Path;

/// The translation catalogs embedded in the app: each language is a folder of `.tsv` files in
/// `locales`, one per part of the interface, so they can be edited apart.
const LOCALES: [&str; 6] = ["de", "fr", "es", "pt-BR", "ja", "zh-Hans"];

fn main() {
    write_catalog_list();
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

/// Writes `catalogs.rs` into the build folder: for each language, the embedded text of its files
/// in name order.
fn write_catalog_list() {
    let root = Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets the manifest folder")).join("locales");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut out = String::new();
    for code in LOCALES {
        let folder = root.join(code);
        println!("cargo:rerun-if-changed={}", folder.display());
        let mut files: Vec<_> = std::fs::read_dir(&folder)
            .unwrap_or_else(|error| panic!("{} cannot be read: {error}", folder.display()))
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|extension| extension == "tsv"))
            .collect();
        files.sort();
        let name = code.replace('-', "_").to_uppercase();
        let _ = write!(out, "pub const {name}: &[(&str, &str)] = &[");
        for file in files {
            println!("cargo:rerun-if-changed={}", file.display());
            let stem = file.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_string();
            let _ = write!(out, "({stem:?}, include_str!({:?})),", file.display().to_string());
        }
        out.push_str("];\n");
    }
    let target = Path::new(&std::env::var("OUT_DIR").expect("cargo sets the build folder")).join("catalogs.rs");
    std::fs::write(target, out).expect("the catalog list can be written");
}
