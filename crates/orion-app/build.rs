//! Gives the Windows executable its icon, which Explorer and the taskbar show.

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/windows/orion.ico");

    // Build scripts run on the machine doing the build, so this only exists there.
    #[cfg(windows)]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../packaging/windows/orion.ico");
        // The icon is decoration: a build without it is better than no build.
        if let Err(error) = resource.compile() {
            println!("cargo:warning=could not embed the Windows icon: {error}");
        }
    }
}
