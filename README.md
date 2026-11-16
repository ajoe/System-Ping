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
cargo test                   # 8 Unit-Tests + 43 Integrationstests (ohne GUI)
cargo clippy --all-targets   # sauber
```

Die Tests brauchen kein Internet: sie pingen `127.0.0.1` mit dem echten `ping.exe`.

### Symbol der Programmdatei

`app/build.rs` hängt die Windows-Ressourcen zur Buildzeit an: `assets/icon.ico` als
Dateisymbol (16/32/48/64/256 px) und ein Versionsblock mit Name, Fassung und Sprache.
Das erledigt die Kiste [`winresource`], die aus einer `.rc` eine `.res` macht; der normale
Linker bettet sie ein, es braucht keinen extra Schritt und kein Nachbearbeiten der `.exe`.

```sh
cargo build --release   # .res entsteht in app/target/release/build/system-ping/*/out/
```

Ist `assets/icon.ico` neuer als die letzte Übersetzung, baut `build.rs` neu
(`cargo:rerun-if-changed`). Das Fenstersymbol zur Laufzeit ist unabhängig davon direkt in
`main.rs` eingebettet (`assets/icon.png`), damit auch `cargo run` ein Symbol zeigt.

[`winresource`]: https://crates.io/crates/winresource

## Benutzen

```
system-ping.exe                       Fenster öffnen
system-ping.exe --export-csv <datei>  gespeicherten Verlauf als CSV exportieren und beenden
system-ping.exe --screenshot <datei.png> [ansicht]  ein Bild der Oberfläche rendern, wie es die Prüfung anfordert (alias: --shot)
system-ping.exe --shot <png> [ansicht]   ein Bild der Oberfläche rendern (chart|stats|events|settings|edit)
system-ping.exe --theme <hell|dunkel>  Aussehen für diese Sitzung überschreiben
system-ping.exe --collect <sekunden>   Fenster kurz öffnen, messen, Zahlen ausgeben, beenden
```

Das Bild kann auch ein anderes Zeitfenster zeigen, ohne die gespeicherten Einstellungen
anzufassen: `--shot tagesbild.png 24h` (auch „12 h“, „30 min“, „90 s“).

Ziele werden über das Einstellungsfeld angelegt (Name + Host). Der Standard-Gateway wird
beim ersten Start aus `ipconfig`/`route` geraten. Ein **Klick** auf ein Ziel in der
linken Liste öffnet den Inline-Editor: Name, Adresse, Kategorie und ob gemessen wird. Enter
oder „✓ Speichern“ übernimmt, Escape oder „✗ Abbrechen“ verwirft. Der Editor zeigt außerdem
die Details des Ziels (Messwerte, ø, min/max, Verluste im sichtbaren Zeitfenster). Ein zweiter
Klick auf dasselbe Ziel lässt den Editor samt Eingabe stehen; schließen tun nur Abbrechen,
Escape, Speichern oder ein Klick auf eine andere Zeile — dabei wird vorher gespeichert.
**Entfernen** gibt es im Editor links oben („entfernen“), also am Ziel, an dem man gerade
arbeitet — nicht mehr oben rechts am markierten Eintrag. In den Einstellungen gibt es deshalb
keine Ziel-Gruppe mehr; was dort übrig blieb (das Löschen des Verlaufs), heißt jetzt **„Messwerte
löschen“** bei den Daten und betrifft ausdrücklich alle Ziele.

Jedes Ziel bekommt automatisch eine eigene Linienfarbe aus einer Palette von acht Farben
(Blaugrün, Blau, Violett, Orange, Grün, Zyan, Rot, Gelb); im Diagramm, in der Legende, in der
Liste links und in den Sparklines ist es immer dieselbe. Der Farb-Knopf im Editor stellt sie um:
eine Farbe wählen oder mit „Auto“ die automatische wieder übernehmen. Was man wählt, wird als
Nummer der Palette gespeichert und überlebt einen Neustart; ein Ziel, das per Hand gewählt ist,
gibt seine Farbe für die automatischen frei. Sind mehr Ziele als Farben da, werden die Farben
wieder verwendet. Die kleine Scheibe vor dem Namen bleibt davon unberührt — sie zeigt weiter den
Zustand (grün/gelb/orange/rot).

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

## Sehen, was man messen will

Ob ein Ziel im Diagramm, in der Legende, in der Ereignisliste und in der Statistik steht, wird an
einer Stelle entschieden: es muss den Textfilter oben passieren (er sucht in Name, Adresse **und**
Kategorie) und die Kategorie muss an sein.

* **Kategorien als Schalter.** In der zweiten Zeile der Kopfzeile hängt pro benutzter Kategorie
  ein Knopf („Gateway“, „LAN-Gerät“, „DNS“, …). Ausgeschaltet verbirgt er alle Ziele dieser
  Kategorie in allen Ansichten — gemessen wird weiter, es fehlt nur die Darstellung. Der Knopf
  „alle“ blendet alles wieder ein; die Wahl wird gespeichert.
* **Einzelne Ziele per Klick in der Legende.** Ein Klick auf den Namen eines Ziels in der Legende
  oben im Diagramm blendet genau dieses eine Ziel aus, ein zweiter holt es zurück. Ausgeblendete
  Ziele stehen weiterhin in der Legende — schwach geschrieben und mit einem hohlen Farbfleck statt
  einer vollen Scheibe —, sonst gäbe es kein Zurück. Sie fehlen nur in der Statistik-Tabelle, die
  keinen Schalter zum Zurückholen hat.

## Im Diagramm blättern und zoomen

Neben „◀ zurück“/„vor ▶“ und den Sprungknöpfen zu Ausreißern (⚑) kann das Diagramm selbst bedient
werden:

* **Mausrad über dem Diagramm** zoomt in das Zeitfenster hinein bzw. wieder heraus, und zwar um die
  Stelle unter dem Zeiger herum. Das Ergebnis ist immer eine der Voreinstellungen aus der Liste
  oben (5 s bis 24 h), damit die Achse ein Name bleibt, den auch die ComboBox zeigen kann. Zoomen
  verlässt „Live“.
* **Doppelklick ins Diagramm** geht zurück an das Ende: Fenster wird auf „Live“ gesetzt.

## Diagramm über 24 Stunden

Das Zeitfenster des Diagramms ist eine Voreinstellung, kein Schieberegler — von 15 s bis
**24 h** (oben in der Leiste und in den Einstellungen). Ab zwei Stunden werden die Achsen mit
der Uhrzeit beschriftet, sonst mit Minuten und Sekunden.

Damit ein Tag kein Loch im Diagramm ist, wächst die Zahl der Messwerte, die ein Ziel behält,
automatisch mit dem Fenster: „Max. Messwerte je Ziel“ steht deshalb auf **auto** (0). Bei 24 h
und einer Sekunde Abstand sind das rund 260 000 Werte pro Ziel (etwa 4 MiB); wer eine Zahl
einträgt, bekommt sie, alles unter 1 024 wird auf diese Untergrenze gehoben.

## Zustand und Dateien

* Speicherverzeichnis: `%LOCALAPPDATA%\system-ping\`, überbar mit der Umgebungsvariablen
  `SYSTEM_PING_STATE_DIR`.
* `system-ping-state.json` — Optionen plus Verlauf. Version 2 speichert jeden Messwert mit
  absoluter Unix-Zeit in Millisekunden; Dateien von Version 1 (Sekunden seit Prozessstart)
  werden beim Laden auf eine echte Zeitachse gehoben.
* Dieselbe Datei merkt sich die **Fenstergröße und -position** (Feld `window`, innere Position in
  physikalischen Pixeln, Größe in logischen Punkten). Beim nächsten Start liegt das Fenster wieder
  da, wo es war; unplausible Werte (Null oder winzig) werden verworfen und es gibt die
  Standardgröße 1080×700. Ein `--shot` ändert die gespeicherte Lage nicht.
* CSV-Export: `timestamp,target,latency_ms,status` mit lokalen ISO-8601-Stempeln. „CSV exportieren“
  schreibt in den Zustandsordner, „Speichern unter…“ fragt vorher nach einem Ziel; beides läuft im
  Hintergrund, damit das Fenster beim Antworten nicht stehenbleibt. Eine optionale Protokoll-Datei
  schreibt jeden Messwert sofort fortlaufend mit.

## Was noch fehlt

Drei Optionen in den Einstellungen sind gespeichert, aber nicht umgesetzt und deshalb
abgeschaltet: Ton bei Ausfall/Wiederherstellung, Symbol im Infobereich, minimiert starten.

## Ordner

* `app/` — Crate (Quellcode in `app/src`, GUI-freie Tests in `app/tests`, `build.rs` für die
  Windows-Ressourcen)
* `assets/` — `icon.ico` (Dateisymbol) und `icon.png` (Fenstersymbol zur Laufzeit)
* `reference/` — Screenshots des Vorbilds und eigene Vergleichsbilder
* `capture/` — Hinweise zum Referenzmaterial
