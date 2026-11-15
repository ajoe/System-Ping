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
cargo test                   # 7 Unit-Tests + 22 Integrationstests (ohne GUI)
cargo clippy --all-targets   # sauber
```

Die Tests brauchen kein Internet: sie pingen `127.0.0.1` mit dem echten `ping.exe`.

## Benutzen

```
system-ping.exe                       Fenster öffnen
system-ping.exe --export-csv <datei>  gespeicherten Verlauf als CSV exportieren und beenden
system-ping.exe --screenshot <datei.png> [ansicht]  ein Bild der Oberfläche rendern, wie es die Prüfung anfordert (alias: --shot)
system-ping.exe --shot <png> [ansicht]   ein Bild der Oberfläche rendern (chart|stats|events|settings|edit)
system-ping.exe --theme <hell|dunkel>  Aussehen für diese Sitzung überschreiben
system-ping.exe --collect <sekunden>   Fenster kurz öffnen, messen, Zahlen ausgeben, beenden
```

Ziele werden über das Einstellungsfeld angelegt (Name + Host). Der Standard-Gateway wird
beim ersten Start aus `ipconfig`/`route` geraten. Ein **Klick** auf ein Ziel in der
linken Liste öffnet den Inline-Editor: Name, Adresse, Kategorie und ob gemessen wird. Enter
oder „✓ Speichern“ übernimmt, Escape oder „✗ Abbrechen“ verwirft. Der Editor zeigt außerdem
die Details des Ziels (Messwerte, ø, min/max, Verluste im sichtbaren Zeitfenster).

Das Fenster startet **nicht** von selbst mit dem Messen — erst „▶ Alle starten“ (oder der
einzelne Knopf pro Ziel) füllt die Diagramme. `--collect` macht das automatisch und ist der
Weg, die Messung ohne Blick auf den Bildschirm zu prüfen:

```
$ app\target\release\system-ping.exe --collect 10
collect: Gateway        192.168.178.1        samples=5 antworten=5 letzte=1.0 ms
collect: DNS (Google)   8.8.8.8              samples=5 antworten=5 letzte=13.0 ms
collect: Internet       1.1.1.1              samples=5 antworten=5 letzte=10.0 ms
collect: 15 Messwerte in dieser Zeit, davon 15 Antworten
```

Der Lauf speichert sein Ergebnis, also zeigt `--export-csv` direkt danach dieselben Werte.

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
