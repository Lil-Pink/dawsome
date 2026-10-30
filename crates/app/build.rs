//! Gives dawsome-app.exe its Windows resources from dawsome-app.rc: the icon Explorer and the
//! taskbar show, and the version details of the file's Properties page. Other targets get
//! nothing from this script.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // The target's system, not this script's: a cross-build runs the script on Linux.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=dawsome-app.rc");
    println!("cargo:rerun-if-changed=../../assets/icon/dawsome.ico");
    let macros = [
        concat!("VERSION=\"", env!("CARGO_PKG_VERSION"), "\""),
        concat!("VERSION_MAJOR=", env!("CARGO_PKG_VERSION_MAJOR")),
        concat!("VERSION_MINOR=", env!("CARGO_PKG_VERSION_MINOR")),
        concat!("VERSION_PATCH=", env!("CARGO_PKG_VERSION_PATCH")),
    ];
    // Required, not optional: a release without its icon must fail to build rather than ship.
    // Both toolchains the Windows build already needs bring a resource compiler (rc.exe with
    // the Windows SDK, llvm-rc with the LLVM tools a cross-build uses).
    embed_resource::compile("dawsome-app.rc", macros)
        .manifest_required()
        .unwrap_or_else(|failure| panic!("dawsome-app.exe's resources were not built: {failure}"));
}
