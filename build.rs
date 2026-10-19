fn main() {
    println!("cargo:rerun-if-changed=assets/windows.rc");
    println!("cargo:rerun-if-changed=assets/leetdesk.ico");
    #[cfg(windows)]
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
        // GPUI startup and editor layout exceed the default 1 MiB main stack
        // in unoptimized Windows builds. Reserve room without committing it all.
        println!("cargo:rustc-link-arg-bin=leetdesk=/STACK:8388608");
        // GPUI's Windows backend loads the application's icon resource ID 1.
        embed_resource::compile_for("assets/windows.rc", ["leetdesk"], embed_resource::NONE)
            .manifest_required()
            .expect("无法编译 Windows 图标资源，请检查 Windows SDK");
    }
}
