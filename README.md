# System-Ping Open Source

[English version: README.en.md](README.en.md)

Ein kleiner Netzwerk-Latenz-Monitor für Windows, Linux und macOS: mehrere Ziele im Intervall
anpingen, Verlauf pro Ziel behalten, Live-Diagramm, Statistik, Statusliste, Alarme und CSV-Export.

Gebaut als Clean-Room-Nachbau: die Oberfläche orientiert sich an Screenshots eines
kommerziellen Werkzeugs, der Code ist vollständig eigenständig (Rust + [egui/eframe]).

Die Oberfläche ist zweisprachig: **Deutsch und Englisch**, umschaltbar in den Einstellungen
unter „Sprache“ (oder einmalig per `--lang en`). Alle Texte kommen aus einer einzigen Tabelle
(`app/src/i18n.rs`), die Wahl wird mit den übrigen Optionen gespeichert.

## Bauen

Voraussetzung ist ein normales Rust-Toolchain-Setup (`rustup`). Auf Windows ist das der
MSVC-VS-Build; auf Linux werden neben dem Compiler auch die X11-/Wayland-Entwicklungsdateien
gebraucht, weil `eframe` direkt gegen die Systembibliotheken bindet:

```sh
# Debian/Ubuntu
sudo apt install build-essential pkg-config libgl1-mesa-dev libx11-dev libxcursor-dev \
                 libxi-dev libxinerama-dev libxrandr-dev libwayland-dev libxkbcommon-dev
# Fedora
sudo dnf install gcc mesa-libGL-devel libX11-devel libXcursor-devel libXi-devel \
                 libXinerama-devel libXrandr-devel wayland-devel libxkbcommon-devel
```

macOS braucht nur die Command Line Tools (`xcode-select --install`). Unter Windows enthält
`app/.cargo/config.toml` nur einen Unterdrückungsfilter für eine irreführende Linker-Warnung von
`ring`; die Datei kann gelöscht werden, das Projekt baut dann unverändert.

```sh
cd app
cargo build --release        # -> app/target/release/system-ping(.exe)
cargo test                   # 104 Tests (38 Unit- + 66 Integrationstests, ohne GUI)
cargo clippy --all-targets   # sauber
```

Andere Systeme lassen sich von Hand prüfen (Typ-Check, ohne deren Bibliotheken zu binden):

```sh
rustup target add x86_64-apple-darwin x86_64-unknown-linux-gnu
cargo check --target x86_64-unknown-linux-gnu --profile test --lib   # warntfrei
```

Die Tests brauchen kein Internet: sie pingen `127.0.0.1` mit dem `ping` des Systems.

### Plattformunterschiede

Alles, was ein Betriebssystem anders macht, sitzt abgesondert in `app/src/platform/` — die
übrige App fragt nie, auf welchem System sie läuft:

| | Windows | Linux | macOS |
| --- | --- | --- | --- |
| Zustandsordner | neben der `.exe` (portabel) | `$XDG_DATA_HOME/system-ping`, sonst `~/.local/share/system-ping` | `~/Library/Application Support/system-ping` |
| Ping-Aufruf | `ping -n 1 -w <ms>` | `ping -c 1 -W <s>` | `ping -c 1 -W <s>` |
| Gateway erraten | `route print`, sonst `ipconfig` | `ip route show`, sonst `netstat -rn` | `route -n get default`, sonst `netstat -rn` |
| Autostart | `HKCU\...\Run` | nicht angeboten | nicht angeboten |
| Dateisymbol | in die `.exe` eingebettet | `.desktop`-Eintrag (nicht umgesetzt) | `.icns` im Bundle (nicht umgesetzt) |

Fehlende Funktionen sagen `false`/`None` statt so zu tun, als gäbe es sie: Der Autostart-Knopf
in der Kopfzeile und das Kontrollkästchen in den Einstellungen erscheinen auf Linux und macOS
gar nicht erst, und die Einstellungen erklären stattdessen, dass dieser Punkt dort nicht
unterstützt wird. `SYSTEM_PING_STATE_DIR` setzt den Zustandsordner auf jedem System.

### Symbol der Programmdatei

Nur Windows kennt ein in die Programmdatei eingebettetes Symbol: `app/build.rs` hängt die
Ressourcen zur Buildzeit an — `assets/icon.ico` als Dateisymbol (16/32/48/64/256 px) plus ein
Versionsblock. Das erledigt die Kiste [`winresource`] aus einer `.rc`; der normale Linker bettet
sie ein, es braucht keinen extra Schritt und kein Nachbearbeiten der `.exe`. Auf Linux und macOS
macht `build.rs` deshalb nichts — dort reist das Symbol als Datei neben dem Programm.

```sh
cargo build --release   # .res entsteht in app/target/release/build/system-ping/*/out/
```

Das Fenstersymbol zur Laufzeit ist unabhängig davon direkt in `main.rs` eingebettet
(`assets/icon.png`), damit auch `cargo run` auf jedem System ein Symbol zeigt.

[`winresource`]: https://crates.io/crates/winresource

## Benutzen

Das Programm heißt `system-ping` (auf Windows `system-ping.exe`). Die Beispiele unten stehen in
Windows-Schreibweise; auf Linux und macOS einfach `./target/release/system-ping` dafür lesen.

```
system-ping.exe                       Fenster öffnen
system-ping.exe --help                Übersicht über alle Argumente (alias: -h)
system-ping.exe --lang <de|en>        Sprache der Oberfläche für diesen Start, ohne sie zu speichern
system-ping.exe --export-csv <datei>  gespeicherten Verlauf als CSV exportieren und beenden
system-ping.exe --screenshot <datei.png> [ansicht]  ein Bild der Oberfläche rendern, wie es die Prüfung anfordert (alias: --shot)
system-ping.exe --shot <png> [ansicht]   ein Bild der Oberfläche rendern (chart|stats|events|settings|edit)
system-ping.exe --theme <hell|dunkel>  Aussehen für diese Sitzung überschreiben
system-ping.exe --collect <sekunden>   Fenster kurz öffnen, messen, Zahlen ausgeben, beenden
```

`--help` antwortet in der Sprache des Starts: ohne `--lang` auf Deutsch, mit `--lang en` auf
Englisch. Das ist die vollständige Ausgabe des Programms selbst, nicht eine Abschrift:

```
System-Ping Open Source — Latenz-Monitor

Ohne Argumente öffnet sich das Fenster mit dem letzten Zustand (Ziele, Einstellungen,
Fenstergröße). Messwerte werden alle 8 s automatisch in den Zustandsordner geschrieben.

Argumente:
  --help, -h              diese Übersicht zeigen und beenden
  --lang <de|en>          Sprache der Oberfläche für diesen Start, ohne sie zu speichern
  --theme <light|dark>    Aussehen für diesen Start erzwingen, ohne es zu speichern
  --collect <sekunden>    im Fenster messen, berichten und beenden (Selbsttest)
  --export-csv [datei]    gespeicherte Messwerte als CSV schreiben und beenden
                          (ohne Dateiname: Standardordner, Zeitstempel im Namen)
  --shot <datei.png> [ansicht]
                          Bild der Oberfläche rechnen und beenden; Ansichten: chart,
                          events, stats, settings, edit, dazu ein Zeitfenster wie 24h

Umgebungsvariablen:
  SYSTEM_PING_STATE_DIR   Zustandsordner (Dateien, Protokoll, Standard-Export) erzwingen
  SYSTEM_PING_MAX_FRAMES  Bilderdurchlauf nach n Bildern abbrechen (nur für Testläufe)
```

Das Bild kann auch ein anderes Zeitfenster zeigen, ohne die gespeicherten Einstellungen
anzufassen: `--shot tagesbild.png 24h` (auch „12 h“, „30 min“, „90 s“). Ansicht und Fenster
stehen direkt nach der Datei, also `--shot bild.png stats 6h --theme dunkel`; ein Optionswort
wie `--theme` beendet die Ansichtsliste sonst — andernfalls landete das Bild auf dem Diagramm.

Ziele werden über das Einstellungsfeld angelegt (Name + Host). Der Standard-Gateway wird
beim ersten Start aus der Routing-Tabelle des Systems geraten (`route print`/`ipconfig`,
`ip route show`, `route -n get default`). Ein **Klick** auf ein Ziel in der
linken Liste öffnet den Inline-Editor: Name, Adresse, Kategorie und ob gemessen wird. Enter
oder „✓ Speichern“ übernimmt, Escape oder „✗ Abbrechen“ verwirft. Der Editor zeigt außerdem
die Details des Ziels (Messwerte, ø, min/max, Verluste im sichtbaren Zeitfenster). Ein zweiter
Klick auf dasselbe Ziel lässt den Editor samt Eingabe stehen; schließen tun nur Abbrechen,
Escape, Speichern oder ein Klick auf eine andere Zeile — dabei wird vorher gespeichert.
**Entfernen** gibt es im Editor links oben („entfernen“), also am Ziel, an dem man gerade
arbeitet — nicht mehr oben rechts am markierten Eintrag.

Ein Ziel anzulegen geht an zwei Stellen: oben in der Leiste und, für den Moment beim Lesen der
Liste, in den Einstellungen unter **„Ziele“** — mit denselben beiden Feldern und Enter als
Bestätigung. Daneben stehen drei Vorschlags-Knöpfe (Google DNS, Cloudflare, Quad9), die das Ziel
nicht nur eintragen, sondern sofort mit dem Messen anfangen; sonst steht eine Zeile in der Liste,
die niemand misst, und die erste Frage ist, warum das Diagramm leer bleibt. Was dort früher übrig
blieb (das Löschen des Verlaufs), heißt jetzt **„Messwerte löschen“** bei den Daten und betrifft
ausdrücklich alle Ziele.

Jedes Ziel bekommt automatisch eine eigene Linienfarbe aus einer Palette von acht Farben; im
Diagramm, in der Legende, in der Liste links und in den Sparklines ist es immer dieselbe. Der
Farb-Knopf im Editor stellt sie um: eine Farbe wählen oder mit „Auto“ die automatische wieder
übernehmen. Was man wählt, wird als Nummer der Palette gespeichert und überlebt einen Neustart;
ein Ziel, das per Hand gewählt ist, gibt seine Farbe für die automatischen frei. Sind mehr Ziele
als Farben da, werden die Farben wieder verwendet. Die kleine Scheibe vor dem Namen bleibt davon
unberührt — sie zeigt weiter den Zustand (grün/gelb/orange/rot).

**Reihenfolge selbst bestimmen:** eine Zeile der linken Liste fassen und halten (linksklick,
Maus gedrückt lassen), dann nach oben oder unten ziehen. Die Liste räumt dabei sofort unter dem
Zeiger um, die gezogene Zeile ist ausgegraut, und der Mauszeiger zeigt die greifende Hand; ein
Greifpunkt aus sechs Punkten am linken Rand markiert die Zeile. Loslassen entscheidet den end-
gültigen Platz — oberhalb der Mitte einer Zeile landet man vor ihr, darunter hinter ihr, und in
der Lücke zwischen zwei Zeilen direkt an dieser Grenze. Die Reihenfolge ist dieselbe, die im
Zustandsfile steht, also nach einem Neustart wieder da. Ein Ziel, dessen Editor offen ist, lässt
sich nicht ziehen: das Formular hat Vorrang, und ein Drop auf ihm wird absichtlich verworfen,
sonst würde ein Feld des Formulars getroffen. Ob eine Zeile Klick oder Zug ist, entscheidet dabei
egui selbst — die Hand darf beim Ziehen über sechs Punkte vom Ausgangspunkt abweichen, sonst war es
ein Klick. Wer zieht, öffnet dabei also keinen Editor, und wer beim Klicken leicht abrutscht,
verschiebt noch nichts. `app/tests/drag_order.rs` prüft beide Fälle mit je zwei verschiedenen Punk-
ten für Druck und Loslassen; einer der Tests (`a_drag_released_far_from_where_it_began_...`) ist
genau wegen des Abrutschens schon einmal durchgerutscht, weil dort beide Male derselbe Punkt ge-
nommen wurde.

## Sprache: eine Tabelle, zwei Sprachen

Alle Texte der Oberfläche hängen in `app/src/i18n.rs`: pro Text ein Eintrag für Deutsch und einer
für Englisch, gruppiert nach dem Ort, an dem er erscheint. Aus dieser Tabelle erzeugt ein Makro
pro Text eine Methode (`lang.chart()`, `lang.tip_sort_fmt()`), und jede Stelle im Programm holt
sich ihren Text darüber — deshalb braucht ein Sprachwechsel keinen Neustart und keinen Aufbau: die
nächste Runde fragt einfach erneut nach.

Bewusst **nicht** übersetzt werden Dinge, die Daten sind statt Oberfläche: die Protokoll-Datei und
die Ereigniszeilen, die sie schreibt, bleiben Deutsch (ein Export, der beim Umstellen seine eigene
Vergangenheit umetikettiert, wäre schwer zu lesen), ebenso die CSV-Kopfzeile
(`timestamp,target,latency_ms,status`) und die Platzhalter in den Adressfeldern. Die Kategorie- und
Ansichts-Namen aus `TargetKind::label()` / `View::label()` sind ebenfalls Deutsch geblieben — sie
dienen als beständige Kennung in Protokoll und Screenshot-Argumenten; was man sieht, kommt aus
`Lang::kind()` bzw. `Lang::view()`.

Neue Texte anlegen heißt: einen Arm in `texts!` schreiben (je einer für `de` und `en`) **und** den
Namen in die Liste von `accessors!` eintragen. Fehlt der Name dort, fehlt die Methode; wird er
doppelt eingetragen, scheitert der Build — beides also mit einer Fehlermeldung, nicht still.

## Schrift: immer dasselbe Schwarz bzw. Weiß

Alle Texte folgen einer einzigen Regel: im hellen Aussehen **schwarz**, im dunklen **weiß** — über
Labels, Knöpfe, Überschriften, Eingabefelder, Achsenbeschriftung und das vom Maler gezeichnete
Diagramm hinweg. egui bringt das anders mit (seine hellen Knöpfe sind grau 60, seine Labels grau 80,
im dunklen Aussehen grau 140 und 180), deshalb schreibt `theme::apply` die eine Farbe in alle fünf
Widget-Zustände; ein starker Text ist dadurch wirklich weiß bzw. schwarz und nicht nur heller grau.

Was leiser aussehen soll — ausgeblendete Ziele, Achsen, Hinweistext — ist dasselbe Schwarz bzw.
Weiß mit weniger Stärke, nie ein eigenes Grau. Nur zwei Dinge sind bewusst gefärbt: die Akzente
(Ausfall rot, Warnung amber, gut grün, die Linienfarbe eines Ziels) und der markierte Text, der auf
blauer Markierung die Gegenfarbe braucht. `app/tests/textcolor.rs` prüft die Regel.

Eine egui-Eigenart ist dabei im Blick zu behalten: `RichText::weak()` setzt die Farbe **im Text
selbst**, noch bevor das Widget malt — die vom Widget geholt Farbe füllt nur Löcher. Ein schwach
geschriebenes auswählbares Label (die Ansichts-Knöpfe oben, die Kategorie-Chips) blieb deshalb grau,
während der „▶ Alle starten“-Knopf daneben korrekt schwarz war. Deshalb tragen solche Knöpfe in
`ui::view_text` keinen `weak()`: was aktiv ist, sagt der Auswahlhintergrund und nicht eine blassere
Schrift.

Das Fenster startet **nicht** von selbst mit dem Messen — erst „▶ Alle starten“ (oder der
einzelne Knopf pro Ziel) füllt die Diagramme. `--collect` macht das automatisch und ist der
Weg, die Messung ohne Blick auf den Bildschirm zu prüfen:

```
$ app\target\release\system-ping.exe --collect 10
collect: Gateway        192.168.178.1        samples=5 replies=5 last=1.0 ms
collect: DNS (Google)   8.8.8.8              samples=5 replies=5 last=13.0 ms
collect: Internet       1.1.1.1              samples=5 replies=5 last=10.0 ms
collect: 15 measurements in this time, 15 of them replies
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
  einer vollen Scheibe —, sonst gäbe es kein Zurück; der Knopf „◉“ am rechten Rand derselben Zeile
  blendet dasselbe Ziel wieder ein. Sie fehlen in Diagramm, Statusliste und Statistik-Tabelle, und
  die Gesamtzeile rechnet sie nicht mit.
* **Nichts in der Auswahl** ist kein leerer Bildschirm: Das Diagramm sagt, woran es liegt (Textfilter,
  alle Ziele ausgeblendet, Kategorie aus), die Statistik-Tabelle schreibt „Kein Ziel in der Auswahl“.

## Im Diagramm blättern und zoomen

Neben „◀ zurück“/„vor ▶“ und den Sprungknöpfen zu Ausreißern (⚑) kann das Diagramm selbst bedient
werden:

* **Mausrad über dem Diagramm** zoomt in das Zeitfenster hinein bzw. wieder heraus, und zwar um die
  Stelle unter dem Zeiger herum. Das Ergebnis ist immer eine der Voreinstellungen aus der Liste
  oben (5 s bis 24 h), damit die Achse ein Name bleibt, den auch die ComboBox zeigen kann. Zoomen
  verlässt „Live“.
* **Doppelklick ins Diagramm** geht zurück an das Ende: Fenster wird auf „Live“ gesetzt.

## Tastatur

Die Kürzel stehen auf den Ansichts-Knöpfen und sind in den Einstellungen unter „Tastatur“
aufgezählt. Sie greifen nur, wenn kein Eingabefeld aktiv ist und keine Modifikator-Taste gehalten
wird — sonst täten sie in einem Textfeld genau das Falsche.

| Taste | Wirkung |
| --- | --- |
| `1` `2` `3` `4` | Diagramm · Statusliste · Statistiken · Einstellungen |
| `Leertaste` | alle Ziele messen oder anhalten |
| `←` / `→` | im Zeitfenster zurück / vor blättern (ein Viertel des Fensters) |
| `Esc` | offenen Editor abbrechen, sonst zurück ins aktuelle Fenster („Live“) |
| `Enter` | neues Ziel hinzufügen, solange ein Adressfeld den Fokus hat |
| Mausrad | über dem Diagramm: Zeitfenster vergrößern / verkleinern |
| Doppelklick | auf eine Störungszeile: Diagramm springt zu dieser Zeit |

## Messen: Intervall und Zeitfenster

Der Abstand zwischen zwei Prüfungen ist keine freie Zahl, sondern eine Stufe aus
`0,25 · 0,5 · 1 · 2 · 5 · 10 s`. Oben in der Leiste sitzt die einzige Bedienung zur Laufzeit, in den
Einstellungen steht derselbe Wert als lesbare Zeile — zwei Regler für eine Sache waren bisher nur
eine Gelegenheit, sich zu widersprechen. Ein gespeicherter oder getippter Wert landet auf der
nächstliegenden Stufe.

Das Zeitfenster des Diagramms ist eine Voreinstellung (5 s bis 24 h), kein Schieberegler; der
Log-Slider ist aus der Kopfzeile ausgezogen und sitzt jetzt als Preset-Liste bis 24 h dort, wo er
hingehört.

## Statistik-Tabelle

* **Sortieren** durch Klick auf eine Spaltenüberschrift (↑/↓ zeigen die Richtung). Der erste Klick
  sortiert absteigend — „die schlechtesten zuerst“ —, der zweite kehrt um. Sortiert wird nur die
  Reihenfolge der Liste: jedes Ziel behält seine Id und damit seine Farbe.
* **„Gesamt“-Zeile** zählt nur die Ziele zusammen, die gerade in der Auswahl sind (Textfilter,
  Kategorie-Schalter, ausgeblendete Ziele). Median und p95 kommen dabei aus **allen Antworten
  zusammen**, nicht als Mittelwert der Mediane — sonst steht in der Zeile eine Latenz, die kein
  einziges Ziel gemessen hat. Ein Ziel ohne Antwort zieht das Minimum nicht auf 0; ganz ohne
  Antwort heißt das Minimum „—“.
* Die Tabelle ist waagerecht scrollbar, wenn nicht alle Spalten in die Breite passen.

## Diagramm über 24 Stunden

Ab zwei Stunden werden die Achsen mit der Uhrzeit beschriftet, sonst mit Minuten und Sekunden.

Damit ein Tag kein Loch im Diagramm ist, wächst die Zahl der Messwerte, die ein Ziel behält,
automatisch mit dem Fenster: „Max. Messwerte je Ziel“ steht deshalb auf **auto** (0). Bei 24 h
und einer Sekunde Abstand sind das rund 260 000 Werte pro Ziel (etwa 4 MiB); wer eine Zahl
einträgt, bekommt sie, alles unter 1 024 wird auf diese Untergrenze gehoben. Die Sparklines in der
linken Liste schneiden bei der p95-Grenze ihrer eigenen Serie ab (mindestens 40 ms), sonst zieht ein
einzelner Ausreißer jede andere Kurve zur Geraden.

## Ereignisliste

Jede Zeile trägt die Uhrzeit `[HH:MM:SS]` (dieselbe Rechnung wie im CSV-Export), oben steht die
Zeile „N von M“ — gemerkt wird nur, was ins Protokoll passt, gezählt wird alles. Der Schalter
**„nur Störungen“** blendet Wiederherstellungen und Meldungen aus; ein **Doppelklick** auf eine
Störungszeile springt mit dem Diagramm auf diesen Zeitpunkt. Im Diagramm ist die Liste als
Kompaktfassung unter der Kurve eingebettet, in der eigenen Ansicht füllt sie den ganzen Platz und
beginnt beim ältesten Eintrag.

## Statusleiste

Der Zähler „gesichert vor N s“ sagt, wann die Zustandsdatei zuletzt geschrieben wurde (Autosave
läuft alle 8 s); ein eigener „Speichern“-Knopf wäre ein dritter Weg für etwas, das ohnehin
automatisch passiert. `Laufzeit` rechnet ab einer Stunde in `H:MM:SS` um. Der Knopf **„CSV“**
exportiert sofort in den Zustandsordner und öffnet ihn, **„⌂ Daten“** in der Kopfzeile tut nichts
anderes als das Ordnerfenster zeigen.

In der Leiste teilen sich zwei Arten von Sätzen einen Platz: was ein Klick berichtet („3 → 1“
nach dem Umziehen, „Ziel gespeichert“, welcher Pfad ein Export schreibt) und was von selbst
meldet (eine Messung, ein Protokollfile, das nicht mehr schreibbar ist, ein Speichern, das nicht
durchging, die Autostart-Auskunft
aus der Registry). Was der Nutzer zuletzt getan hat, gewinnt — und zwar dauerhaft, nicht nur für
einen Frame. Die automatische Warnung wird dabei nicht weggeworfen: sie kommt wieder, sobald
nichts Neueres da ist, denn sie ist ja noch wahr; ein Protokollfile, das alle 30 s neu
ausprobiert werden darf, durfte eine Aktion nicht eine halbe Minute lang überdecken. Der Grund,
warum das überhaupt geregelt werden musste: solange nichts gemessen wurde, blieb die Leiste auf
ihrem alten Text stehen, und ein Klick ohne laufenden Betrieb sah aus wie gar keiner. Geprüft
wird das in `app/tests/fixes.rs`.

## Zustand und Dateien

* Speicherverzeichnis: auf Windows **neben der Programmdatei** (portabel), auf Linux
  `$XDG_DATA_HOME/system-ping` bzw. `~/.local/share/system-ping`, auf macOS
  `~/Library/Application Support/system-ping`. Auf jedem System über die Umgebungsvariable
  `SYSTEM_PING_STATE_DIR` erzwingbar.
* `system-ping-state.json` — Optionen plus Verlauf. Version 2 speichert jeden Messwert mit
  absoluter Unix-Zeit in Millisekunden; Dateien von Version 1 (Sekunden seit Prozessstart)
  werden beim Laden auf eine echte Zeitachse gehoben.
* Diese Datei wird **stückweise gelesen und atomar geschrieben**. Ein Teil, der sich nicht lesen lässt
  — ein `options`-Block mit einem Feld, das diese Version nicht kennt, eine Zeile im Verlauf, die keine
  Zeile ist — kostet nur diesen Teil, der Rest der Datei gilt weiter. Eine Datei, die größer als 32 MiB
  ist, wird nicht gelesen (das ist eher ein vertipptes Zustandsverzeichnis als die eigenen Einstellungen),
  und eine Verlaufszeile, die tiefer verschachtelt ist als das Format je schreibt, wird einzeln
  verworfen. Geschrieben wird über eine Nebendatei mit anschließendem Umbenennen: Wer beim Speichern
  abstürzt oder den Strom verliert, findet beim nächsten Start die bisher vollständige Datei — die
  halbfertige war noch nicht am richtigen Namen. Geht ein Speichern dagegen schief, weil die Datei
  von einem anderen Programm gehalten wird oder das Verzeichnis fehlt, sagt das die Statusleiste
  („Speichern fehlgeschlagen: …“), und die „zuletzt gespeichert vor …“-Zeit bleibt stehen: Sie soll
  nur eine Datei bestätigen, die wirklich da ist. `app/tests/save_status.rs` prüft beide Fälle.
* Dieselbe Datei merkt sich die **Fenstergröße und -position** (Feld `window`, innere Position in
  physikalischen Pixeln, Größe in logischen Punkten). Beim nächsten Start liegt das Fenster wieder
  da, wo es war; unplausible Werte (Null oder winzig) werden verworfen und es gibt die
  Standardgröße 1080×700. Ein `--shot` ändert die gespeicherte Lage nicht.
* Dieselbe Datei merkt sich auch die **Sprache** (Feld `language`, `de` oder `en`). Ein `--lang`
  auf der Befehlszeile gilt nur für diesen Start und schreibt nichts zurück.
* CSV-Export: `timestamp,target,latency_ms,status` mit lokalen ISO-8601-Stempeln. „CSV exportieren“
  schreibt in den Zustandsordner, „Speichern unter…“ fragt vorher nach einem Ziel; beides läuft im
  Hintergrund, damit das Fenster beim Antworten nicht stehenbleibt. Die Statusleiste nennt die Datei,
  auf die man wartet, und später ihr Ergebnis. Eine optionale Protokoll-Datei
  schreibt jeden Messwert sofort fortlaufend mit. Ein Zielname, der mit `=`, `+`, `-` oder `@` beginnt,
  steht im Export mit vorangestelltem `'` — sonst macht Excel aus der Zelle eine Formel, und die
  Zielnamen sind freie Eingabe. Anführungszeichen und Zeilenumbrüche in einem Namen werden escaped,
  damit ein Messwert eine Zeile bleibt.

## Was noch fehlt

Drei Felder sind in der Zustandsdatei gespeichert, aber bewusst ohne Knopf, weil dahinter nichts
steht: Ton bei Ausfall/Wiederherstellung, Symbol im Infobereich, minimiert starten. Die Werte bleiben
erhalten, falls eine der Funktionen später kommt.

## Bilder

`app/shots/` hält je einen aktuellen Bildschirm pro Sprache (Deutsch dunkel, Englisch hell),
erzeugt mit `--shot`:

```sh
system-ping.exe --shot app\shots\de-chart.png chart
system-ping.exe --lang en --theme light --shot app\shots\en-chart.png chart
```

## Ordner

* `app/` — Crate (Quellcode in `app/src`, darunter `app/src/platform/` für alles
  Betriebssystem-spezifische, GUI-freie Tests in `app/tests`, `build.rs` für die
  Windows-Ressourcen, Screenshots in `app/shots`)
* `assets/` — `icon.ico` (Dateisymbol) und `icon.png` (Fenstersymbol zur Laufzeit)
* `reference/` — Screenshots des Vorbilds und eigene Vergleichsbilder
* `capture/` — Hinweise zum Referenzmaterial
