//! Embeds the scope icon into `scope.exe` so it shows up in Explorer and on the
//! taskbar (issue #230). Windows-only: on every other platform this is a no-op.

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=installer/icons/scope.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("installer/icons/scope.ico");
        // A failure here should not be fatal for local builds without the
        // Windows SDK resource compiler; the icon is cosmetic.
        if let Err(err) = res.compile() {
            println!("cargo:warning=failed to embed icon: {err}");
        }
    }
}
