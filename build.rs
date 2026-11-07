fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            // GPUI supplies the asInvoker and PerMonitorV2 application manifest.
            .compile()
            .expect("compile Windows application icon and version resource");
    }
}
