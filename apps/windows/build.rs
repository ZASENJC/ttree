fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/icon.ico");
    // Resource compilation is a native Windows gate. Cross cargo check on Mac
    // still checks Rust types without requiring an entire Windows resource SDK.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../../assets/icons/icon.ico")
            .set("ProductName", "TTREE")
            .set("FileDescription", "TTREE — Translation and OCR")
            .set("OriginalFilename", "TTREE.exe")
            .set("ProductVersion", env!("CARGO_PKG_VERSION"))
            .set("FileVersion", env!("CARGO_PKG_VERSION"))
            .compile()
            .expect("compile Windows icon and version resources");
    }
}
