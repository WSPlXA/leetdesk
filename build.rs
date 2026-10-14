fn main() {
    println!("cargo:rerun-if-changed=assets/windows.rc");
    println!("cargo:rerun-if-changed=assets/leetdesk.ico");
    #[cfg(windows)]
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
        // GPUI's Windows backend loads the application's icon resource ID 1.
        embed_resource::compile_for("assets/windows.rc", ["leetdesk"], embed_resource::NONE)
            .manifest_required()
            .expect("无法编译 Windows 图标资源，请检查 Windows SDK");
    }
}
