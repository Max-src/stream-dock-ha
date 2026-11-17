// Puts the app's icon and name into the program file, so its shortcuts and
// entry in Windows look right. A missing resource compiler only costs the
// icon, so it never stops the build.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    // The commit it was built from, shown with the version, so two builds of
    // the same version can be told apart. Looked at again after each commit.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=STREAM_DOCK_HA_COMMIT={commit}");
    if !std::path::Path::new("assets/icon.ico").exists() {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/icon.ico");
    resource.set("ProductName", "Stream Dock HA");
    resource.set("FileDescription", "Stream Dock HA");
    if let Err(e) = resource.compile() {
        println!("cargo:warning=The icon wasn't added: {e}");
    }
}
