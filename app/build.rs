// Builds the Windows resources of the EXE: a file icon in Explorer, plus version and description
// in the file properties. `winresource` generates a .res file from this and hands it to the linker
// (rc.exe or llvm-rc), so the result is exactly what a regular Windows program gets — no need to
// post-process the finished EXE.

fn main() {
    println!("cargo::rerun-if-changed=assets/icon.ico");
    println!("cargo::rerun-if-changed=Cargo.toml");

    let mut res = winresource::WindowsResource::new();
    // The path is relative to the manifest folder (app/), so "app/" is left off here.
    res.set_icon("../assets/icon.ico");
    res.set_language(0x0409); // English (US), so the resource is found reliably
    if let Err(e) = res.compile() {
        println!("cargo::warning=embedding the icon failed: {e}");
    }
}
