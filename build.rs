fn main() {
    println!("cargo:rerun-if-changed=assets/warcry.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    let windows_target = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    if cfg!(windows) && windows_target {
        embed_resource::compile("assets/warcry.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed Windows icon");
    }
}
