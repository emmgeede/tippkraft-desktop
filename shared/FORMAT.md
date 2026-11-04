# Formatvertrag Server → Desktop

Der Server liefert diesen Stand über `GET /api/sync/snapshot` und als Ereignis `snapshot` über `GET /api/sync/stream` (Server-Sent Events). Beide Endpunkte verlangen `Authorization: Bearer <Token>`.

`snippets.example.json` ist das verbindliche Beispiel. Web (`web/test/vertrag.test.ts`) und Desktop testen beide dagegen. Wer das Format ändert, ändert zuerst diese Datei und erhöht `version`.

## Felder

| Feld | Typ | Bedeutung |
|---|---|---|
| `version` | Zahl | Formatversion, aktuell `2`. Der Desktop liest 1 und 2 und lehnt unbekannte ab (behält dann seinen letzten Stand) |
| `sprache` | `"en"` \| `"de"` | Ab F15 (Version 2, optional). Sprache des Accounts für Tray, Popup, Übersicht, Meldungen und Datumsformat. Fehlt sie, gilt die Systemsprache |
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
- Reihenfolge auf dem Desktop: Platzhalter und `{{{{` ersetzen (eingegebene Werte mit `[[` werden zu `[[[[` maskiert), danach Tasten-Tokens auswerten.
