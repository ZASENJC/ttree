fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/icon.ico");
    println!("cargo:rerun-if-changed=../../assets/icons/tray-light.ico");
    println!("cargo:rerun-if-changed=../../assets/icons/tray-dark.ico");
    // Resource compilation is a native Windows gate. Cross cargo check on Mac
    // still checks Rust types without requiring an entire Windows resource SDK.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../assets/icons/icon.ico")
            .set_icon_with_id("../../assets/icons/tray-light.ico", "2")
            .set_icon_with_id("../../assets/icons/tray-dark.ico", "3")
            .set("ProductName", "TTREE")
            .set("FileDescription", "TTREE — Translation and OCR")
            .set("OriginalFilename", "TTREE.exe")
            .set("ProductVersion", env!("CARGO_PKG_VERSION"))
            .set("FileVersion", env!("CARGO_PKG_VERSION"))
            .compile()
            .expect("compile Windows icon and version resources");
    }
}
