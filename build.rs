fn main() {
    // Check the *target* OS (not the host's), so cross-compiling to Windows embeds the icon too.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("yamada.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed yamada.ico: {e}");
        }
    }
}
