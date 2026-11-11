fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../../packaging/icons/tabby-rust.ico");
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../packaging/icons/tabby-rust.ico");
        resource.set("FileDescription", "Tabby Rust terminal emulator");
        resource.set("ProductName", "Tabby Rust");
        resource.set("LegalCopyright", "Apache-2.0");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
