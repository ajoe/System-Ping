# System-Ping Open Source

Ein kleiner Netzwerk-Latenz-Monitor für Windows: mehrere Ziele im Intervall anpingen,
Verlauf pro Ziel behalten, Live-Diagramm, Statistik, Statusliste, Alarme und CSV-Export.

Gebaut als Clean-Room-Nachbau: die Oberfläche orientiert sich an Screenshots eines
kommerziellen Werkzeugs, der Code ist vollständig eigenständig (Rust + [egui/eframe]).

## Bauen

Voraussetzung ist ein normales Rust-Toolchain-Setup (`rustup`), standardmäßig MSVC auf
Windows. `app/.cargo/config.toml` enthält deshalb nur einen Unterdrückungsfilter für eine
irreführende Linker-Warnung von `ring`; die Datei kann gelöscht werden, das Projekt baut
dann unverändert.

```sh
cd app
cargo build --release        # -> app/target/release/system-ping.exe
cargo test                   # 6 Unit-Tests + 9 Integrationstests (ohne GUI)
cargo clippy --all-targets   # sauber
```

Die Tests brauchen kein Internet: sie pingen `127.0.0.1` mit dem echten `ping.exe`.

## Benutzen

```
system-ping.exe                       Fenster öffnen
system-ping.exe --export-csv <datei>  gespeicherten Verlauf als CSV exportieren und beenden
system-ping.exe --shot <png> [ansicht]   ein Bild der Oberfläche rendern (chart|stats|events|settings)
system-ping.exe --theme <hell|dunkel>  Aussehen für diese Sitzung überschreiben
```

Ziele werden über das Einstellungsfeld angelegt (Name + Host). Der Standard-Gateway wird
beim ersten Start aus `ipconfig`/`route` geraten.

## Zustand und Dateien

* Speicherverzeichnis: `%LOCALAPPDATA%\system-ping\`, überbar mit der Umgebungsvariablen
  `SYSTEM_PING_STATE_DIR`.
* `system-ping-state.json` — Optionen plus Verlauf. Version 2 speichert jeden Messwert mit
  absoluter Unix-Zeit in Millisekunden; Dateien von Version 1 (Sekunden seit Prozessstart)
  werden beim Laden auf eine echte Zeitachse gehoben.
* CSV-Export: `timestamp,target,latency_ms,status` mit lokalen ISO-8601-Stempeln. Eine
  optionale Protokoll-Datei schreibt jeden Messwert sofort fortlaufend mit.

## Was noch fehlt

Drei Optionen in den Einstellungen sind gespeichert, aber nicht umgesetzt und deshalb
abgeschaltet: Ton bei Ausfall/Wiederherstellung, Symbol im Infobereich, minimiert starten.

## Ordner

* `app/` — Crate (Quellcode in `app/src`, GUI-freie Tests in `app/tests`)
* `reference/` — Screenshots des Vorbilds und eigene Vergleichsbilder
* `capture/` — Hinweise zum Referenzmaterial
