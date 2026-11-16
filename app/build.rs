// Baut die Windows-Ressourcen der EXE: Datei-Symbol im Explorer, dazu Version und
// Beschreibung in den Eigenschaften. `winresource` erzeugt daraus eine .res-Datei und
// uebergibt sie dem Linker (rc.exe bzw. llvm-rc), deshalb ist das Ergebnis dasselbe, was
// ein normales Windows-Programm bekommt — kein Nachbearbeiten der fertigen EXE noetig.

fn main() {
    println!("cargo::rerun-if-changed=assets/icon.ico");
    println!("cargo::rerun-if-changed=Cargo.toml");

    let mut res = winresource::WindowsResource::new();
    // Pfad ist relativ zum Manifest-Ordner (app/), also hier ohne "app/".
    res.set_icon("../assets/icon.ico");
    res.set_language(0x0409); // Englisch (USA), damit die Ressource sicher gefunden wird
    if let Err(e) = res.compile() {
        println!("cargo::warning=Symbol-Einbettung fehlgeschlagen: {e}");
    }
}
