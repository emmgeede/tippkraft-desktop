# Formatvertrag Server → Desktop

Der Server liefert diesen Stand über `GET /api/sync/snapshot` und als Ereignis `snapshot` über `GET /api/sync/stream` (Server-Sent Events). Beide Endpunkte verlangen `Authorization: Bearer <Token>`.

`snippets.example.json` ist das verbindliche Beispiel. Web (`web/test/vertrag.test.ts`) und Desktop testen beide dagegen. Wer das Format ändert, ändert zuerst diese Datei und erhöht `version`.

## Felder

| Feld | Typ | Bedeutung |
|---|---|---|
| `version` | Zahl | Formatversion, aktuell `2`. Der Desktop liest 1 und 2 und lehnt unbekannte ab (behält dann seinen letzten Stand) |
| `sprache` | `"en"` \| `"de"` | Ab F15 (Version 2, optional). Sprache des Accounts für Tray, Popup, Übersicht, Meldungen und Datumsformat. Fehlt sie, gilt die Systemsprache |
| `desktopVersion` | Text | Ab F21 (Version 2, optional). Neueste veröffentlichte Desktop-Version, z. B. `0.2.0`. Ist sie neuer als die laufende, zeigt der Desktop einen Update-Hinweis. Fehlt sie, gibt es keinen Hinweis |
| `bausteine` | Liste | Alle Bausteine des Nutzers, sortiert nach `lower(kuerzel)` in Zeichencode-Reihenfolge. Auslösen dürfen nur die mit `aktiv: true` |
| `bausteine[].aktiv` | bool | Ab Version 2. `true`: Baustein aktiv und Gruppe aktiv oder keine. Fehlt (Version 1): `true` |
| `bausteine[].bezeichnung` | Text oder `null` | Ab Version 2, für die Übersicht (F14). Fehlt (Version 1): `null` |
| `bausteine[].kuerzel` | Text | 1 bis 64 Zeichen, keine Leer- oder Steuerzeichen |
| `bausteine[].inhalt` | Text | Einzufügender Text, Zeilenumbruch immer `\n`, keine Steuerzeichen außer `\t` und `\n`. Kann Tasten-Tokens enthalten (siehe unten). Verweise `{{kürzel}}` hat der Server schon aufgelöst. Übrig bleiben Platzhalter und `{{{{` (siehe unten) |
| `bausteine[].ausloeser` | `"sofort"` \| `"leertaste"` \| `"enter"` \| `"tab"` | Was die Ersetzung auslöst. Der Auslöser selbst wird verbraucht, nicht mitgesendet |
| `bausteine[].wortgrenze` | bool | `true`: Vor dem Kürzel muss eine Wortgrenze stehen (Pufferanfang, Leerzeichen, Satzzeichen) |
| `bausteine[].grossKleinBeachten` | bool | `false`: `MFG` löst `mfg` aus |
| `bausteine[].schreibweiseUebernehmen` | bool | Nur bei `grossKleinBeachten: false`. Beginnt das getippte Kürzel mit einem Großbuchstaben (und ist nicht komplett groß), wird der erste Buchstabe des Inhalts großgeschrieben |
| `ausschluesse` | Liste von Text | Wayland-`app_id`s, in denen nichts auslöst. Vergleich exakt, ohne Groß-/Kleinschreibung |

## Tasten-Tokens im Inhalt

- `[[Name]]` oder `[[Name:n]]` mit `n` von 1 bis 20 drückt die Taste `n`-mal.
- Erlaubte Namen (Groß-/Kleinschreibung zählt): `Tab`, `Enter`, `Space`, `Backspace`, `Delete`, `Escape`, `Left`, `Right`, `Up`, `Down`, `Home`, `End`.
- `[[[[` steht für ein wörtliches `[[`. Im Beispiel ergibt `[[[[kein Token]]` den Text `[[kein Token]]`.
- Der Server garantiert, dass jeder Inhalt nur gültige Tokens enthält. Ein `[[` ohne `]]` kommt nicht vor.

## Platzhalter im Inhalt (F8)

Der Desktop füllt sie beim Auslösen aus, der Server prüft nur die Syntax:

- `{{?Name}}`: Textfeld im Popup. `{{?Name:a|b|…}}`: Auswahl (2 bis 20 Optionen, leere Option erlaubt). Gleicher Name wird einmal abgefragt.
- `{{=date}}` (oder `{{=datum}}`): heutiges Datum ohne Popup, bei `sprache: de` als `TT.MM.JJJJ`, sonst ISO `JJJJ-MM-TT`.
- `{{{{` steht für ein wörtliches `{{`. Der Server gibt es unverändert weiter, auch aus eingebundenen Bausteinen.
- `{{=date:FORMAT}}` (F40): Format mit `d`, `dd`, `M` … `MMMM`, `yy`, `yyyy`, `EEE`, `EEEE`, `HH`, `mm`, ab F42 auch `w`/`ww` (ISO-Kalenderwoche) und `D`/`DDD` (Tag des Jahres). Text in `'…'` bleibt wörtlich.
- `{{=clipboard}}`, `{{=cursor}}` (F9): Zwischenablage, Cursorposition.

## Erweiterte Makros (F42)

Formatversion bleibt 2. Ältere Desktops tippen diese Makros als Text.

- `{{=date+14}}`, `{{=date-1m:FORMAT}}`: Datum verschoben, Einheit `d` (Standard), `w`, `m`, `y`, höchstens 4 Ziffern.
- `{{=random:a|b|c}}`: zufällig eine Option, ohne Popup.
- `{{?Name*}}`: Pflichtfeld. `{{?Name:number}}`, `{{?Name:email}}`: Prüfung im Popup.
- `{{$Name}}`: Variable einsetzen, `{{$Name=Text}}`: setzen (fügt nichts ein). Jede Abfrage ist zugleich Variable. Variablen leben nur im Speicher der Desktop-App.
- `{{#if Name}}`, `{{#if Name=Wert}}`, `{{#if Name!=Wert}}` … `{{#else}}` … `{{#end}}`: Bedingung, bis 5 Ebenen. Vergleich ohne Groß/klein und Leerzeichen am Rand. Abfragen in nicht gewählten Zweigen zeigt das Popup nicht. Der Server garantiert, dass Blöcke geschlossen sind.


## Auswahl mit Inhalt, Variable aus Inhalt (F55)

- `{{#choice Name}}{{#option A}}…{{#option B}}…{{#end}}`: Auswahl im Popup mit den Namen der Möglichkeiten (2 bis 20). Eingefügt wird der Inhalt der gewählten Möglichkeit, der wieder alles enthalten darf. `Name` wird zur Variable mit dem Namen der gewählten Möglichkeit. Abfragen in nicht gewählten Möglichkeiten zeigt das Popup nicht.
- `{{#set Name}}…{{#end}}`: Inhalt auswerten und als Text in die Variable schreiben, nichts einfügen. Tasten-Tokens darin werden zu Text.
- Verweise `{{kürzel}}` löst der Server bis zu 5 Ebenen tief auf, Kreise lehnt er beim Speichern ab.
- Reihenfolge auf dem Desktop: Platzhalter und `{{{{` ersetzen (eingegebene Werte mit `[[` werden zu `[[[[` maskiert), danach Tasten-Tokens auswerten.
