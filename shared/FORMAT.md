# Formatvertrag Server → Desktop

Der Server liefert diesen Stand über `GET /api/sync/snapshot` und als Ereignis `snapshot` über `GET /api/sync/stream` (Server-Sent Events). Beide Endpunkte verlangen `Authorization: Bearer <Token>`.

`snippets.example.json` ist das verbindliche Beispiel. Web (`web/test/vertrag.test.ts`) und Desktop testen beide dagegen. Wer das Format ändert, ändert zuerst diese Datei und erhöht `version`.

## Felder

| Feld | Typ | Bedeutung |
|---|---|---|
| `version` | Zahl | Formatversion, aktuell `1`. Der Desktop lehnt unbekannte Versionen ab und behält seinen letzten Stand |
| `bausteine` | Liste | Nur Bausteine, die auslösen dürfen (aktiv und in aktiver oder keiner Gruppe). Sortiert nach `lower(kuerzel)` in Zeichencode-Reihenfolge |
| `bausteine[].kuerzel` | Text | 1 bis 64 Zeichen, keine Leer- oder Steuerzeichen |
| `bausteine[].inhalt` | Text | Einzufügender Text, Zeilenumbruch immer `\n`, keine Steuerzeichen außer `\t` und `\n`. Kann Tasten-Tokens enthalten (siehe unten). Verweise `{{kürzel}}` hat der Server schon aufgelöst. Ein `{{` im Inhalt ist deshalb immer wörtlicher Text |
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
