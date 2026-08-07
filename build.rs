//! Embeds the application icon into the executable, so PetPal has a face in
//! Explorer, the taskbar and Task Manager rather than the generic exe glyph.
//!
//! `assets/petpal.ico` is generated from the built-in creature itself — see the
//! `generate_app_icon` test in `src/sheet.rs`.

fn main() {
    println!("cargo:rerun-if-changed=assets/petpal.rc");
    println!("cargo:rerun-if-changed=assets/petpal.ico");

    // `manifest_optional` keeps a non-Windows or toolchain-less build working;
    // the icon is cosmetic and never worth failing a build over.
    if let Err(e) = embed_resource::compile("assets/petpal.rc", embed_resource::NONE)
        .manifest_optional()
    {
        println!("cargo:warning=icon not embedded: {e}");
    }
}
