fn main() {
    println!("cargo:rerun-if-changed=../conduit-daemon/assets/conduit-icon-dark.ico");
    #[cfg(windows)]
    {
        winres::WindowsResource::new()
            .set_icon("../conduit-daemon/assets/conduit-icon-dark.ico")
            .compile()
            .expect("Could not embed the original Conduit application icon");
    }
}
