# NWNArmory – Projektanalyse

## 1. Was das Programm macht

NWNArmory ist ein Windows-Desktop-Tool (MFC/Visual C++, offenbar aus der NWN-Modding-Ära um 2003)
zur **automatisierten Größenanpassung von Neverwinter-Nights-Modellen (`.mdl`, ASCII-Format) auf
andere Spielrassen**. Man hat üblicherweise die Standard-Modelle für einen menschlichen Charakter
(z. B. Rüstungsteile: Gürtel, Brust, Hals, Becken, Schulter, Bizeps, Unterarm, Hand, Bein, Schienbein,
Fuß) und möchte daraus automatisch passende Varianten für Halbling, Zwerg, Elf, Gnom und Halbork
erzeugen – ohne jedes Teil von Hand in 3ds Max zu skalieren.

Das Tool liest die Quellmodelle textbasiert (ASCII-`.mdl`), wendet pro Körperteil und Rasse fest
definierte Skalierungs-/Rotations-/Translations-Matrizen auf die Vertex- und Textur-Koordinaten an
und schreibt das Ergebnis unter einem neuen Dateinamen (Rassen-Suffix) in ein Zielverzeichnis.

## 2. Architektur-Überblick

```
NWNArmory.cpp/.h        → CWinApp-Einstiegspunkt, startet den Hauptdialog
NWNArmoryDlg.cpp/.h     → Hauptdialog: Dateiauswahl, Zielordner, INI-Datei, "Go"-Button
BDialog.cpp/.h          → Basis-Dialogklasse mit Hintergrundbild (Tile/Center/Stretch)
FolderDialog.cpp/.h     → Ordnerauswahl-Dialog (SHBrowseForFolder-Wrapper)
MyHyperLink.cpp/.h      → Hyperlink-Steuerelement (nur im About-Dialog verwendet)
ProgressWnd.cpp/.h      → Popup-Fortschrittsfenster während der Verarbeitung
ShowTransformsDlg.cpp/.h→ Listet alle geladenen Transform-Gruppen zur Kontrolle auf

NWNGlobals.cpp/.h       → globales Transform-Array, Logging (CLogFile), Pfad-Hilfsfunktionen
Transform.cpp/.h        → CTransform: eine "Regel" (Match-Muster + Matrizen) pro INI-Sektion
Matrix.cpp/.h           → CMatrix: generische, referenzgezählte 4x4/NxM-Matrixklasse
IO.cpp/.h               → CIO: liest ein Quellmodell, prüft Regel-Treffer, schreibt Zielmodell
ini2.cpp/.h             → CIni: dünner Wrapper um die Windows-INI-API

NWNArmory.ini/standard.ini → Konfigurationsdaten: 110 Transform-Sektionen (Rasse × Körperteil)
```

## 3. Ablauf – Schritt für Schritt

1. **Start (`CNWNArmoryApp::InitInstance`)**: öffnet `CNWNArmoryDlg` modal.
2. **`OnInitDialog`**: lädt beim Start automatisch die Transforms aus der Standard-INI
   (`LoadTransforms()`), setzt Hintergrundbild, deaktiviert den "Go"-Button.
3. **Schritt 1 – Transformationsdatei wählen** (`OnBnClickedBtnTransformation`):
   Der Nutzer kann eine andere `.ini` wählen; `LoadTransforms(pfad)` liest sie neu ein.
4. **`LoadTransforms`**:
   - Liest `[Global] nTransforms` (Anzahl Sektionen, gedeckelt auf `MAXTEMPLATES = 1024`).
   - Legt `CTransform transform[nTransforms]` neu an (vorheriges Array wird `delete[]`-t).
   - Für jede Sektion `s0..sN-1`: liest `match`, `substitute`, `scale`, `rotate`, `translate`,
     `minimum`, `maximum` sowie die Texture-Varianten (`tscale`, `trotate`, `ttranslate`,
     `tminimum`, `tmaximum`, `tbitmap`) und `position`. Bei Parse-Fehlern (`sscanf`-Count
     stimmt nicht) wird eine Meldung angezeigt und die Sektion durch Leeren von `match`
     effektiv deaktiviert (`continue`).
   - Öffnet/erstellt bei Bedarf die Logdatei (`NWNArmory.Log`).
5. **Schritt 2 – Quelldateien wählen** (`OnBnClickedBtnSource`): Mehrfachauswahl von `.mdl`-Dateien
   über einen 32-KB-Puffer (`filebuf[32768]`); Dateinamen werden in `m_SourceFiles`
   und die Listbox übernommen. Quellverzeichnis wird aus der ersten Datei abgeleitet.
6. **Schritt 3 – Zielverzeichnis wählen** (`OnBnClickedBtnDestination`): `CFolderDialog`.
7. **Schritt 4 – "Go!"** (`OnBnClickedBtnGo` → `ProcessSourceFile`):
   Für **jede** Quelldatei wird **jede** geladene Transform-Regel durchprobiert:
   - `CIO io(datei, zielverzeichnis)` öffnet die Quelldatei lesend, ermittelt den
     Basisnamen (klein geschrieben) als `mstrSrcModelName`.
   - `io.SetOutFile(transform[i])`:
     - `doesMatch()` prüft den Dateinamen gegen `match` per **Wildcard-Vergleich**
       (`wildcmp`, unterstützt `*` und `?`, selbstgeschriebene Implementierung in `IO.cpp`).
     - Bei Treffer: `doSubstitute()` baut aus `substitute` (ebenfalls mit `?`/`*`-Platzhaltern)
       den neuen Modellnamen, z. B. `pm01_belt001` + Muster `??a*` → `pm01a_belt001`
       (die ersten beiden Zeichen bleiben, drittes Zeichen wird durch `a` ersetzt = Halbling-Suffix).
     - Zieldatei wird angelegt (`CFile::modeCreate|modeWrite`), Ausnahme bei Fehler.
   - `io.ProcessModel(transform[i])` liest die Quelldatei **zeilenweise** und schreibt sie
     transformiert in die Zieldatei (siehe Abschnitt 4).
   - Ein Quellmodell kann so **mehrfach** verarbeitet werden – einmal pro passender Regel
     (z. B. dieselbe Gürtel-Datei erzeugt Halbling-, Zwerg-, Elf-, Gnom- und Halbork-Varianten,
     da fünf Sektionen mit demselben `match`-Muster, aber unterschiedlichem `substitute`
     existieren).
   - Ein `CProgressWnd` zeigt Fortschritt, erlaubt Abbrechen; Ergebnisse/Fehler werden geloggt
     und teils per `AfxMessageBox` gemeldet.

## 4. Kernlogik der Modell-Transformation (`CIO::ProcessModel`)

Das ASCII-`.mdl`-Format wird zeilenweise per Token-Erkennung (erstes Wort, klein geschrieben)
interpretiert (siehe auch `nwn-mdl-format_v5.md` im Projekt):

| Zeilentyp        | Verhalten |
|-------------------|-----------|
| `verts N`         | liest die folgenden `N` Vertex-Zeilen (`ProcessVerts`/`ProcessVert`), wendet pro Vertex `CMatrix`-Multiplikationen mit Skalierung, Rotation, Translation an (nur wenn der Punkt innerhalb `minimum`/`maximum` liegt, sonst unverändert kopiert) |
| `tverts N`        | analog für Textur-Koordinaten, mit eigener Skala/Rotation/Translation (`ProcessTverts`/`ProcessTvert`); kann optional auf ein bestimmtes `bitmap` beschränkt werden (`tbitmap`) |
| `bitmap <name>`   | merkt sich den zuletzt gesehenen Bitmap-Namen (für die `tbitmap`-Prüfung) und ersetzt Modellnamen-Referenzen |
| `position <x y z>`| transformiert den Pivot-Punkt (`ProcessPosition`), entweder mit denselben Scale/Rotate/Translate-Matrizen oder als absolute Verschiebung (`isMovePos`, wenn `position=(x,y,z)` in der INI gesetzt ist) |
| `filedependancy`  | wird unverändert kopiert (Max-Dateireferenz, für den Loader irrelevant) |
| alles andere      | Modellname wird per `ReplaceNoCase` ersetzt, Zeile unverändert übernommen |

Die eigentliche Vektor-Transformation läuft über homogene 4×1-Vektoren (`CMatrix v(4,1)`) und
4×4-Matrizen (Skalierung, Rotation aus Grad in `CTransform::SetRotateFromDegrees`, Translation),
die in `CTransform` einmal beim Laden der INI vorgerechnet werden (`m_rotTransform` etc.).
Rotation wird dabei explizit für ein **linkshändiges Koordinatensystem** (3ds Max) negiert
(Kommentar in `Transform.cpp`: "Max uses a LH-coordinate system").

`CMatrix` ist eine klassische Copy-on-Write-Matrixklasse: Kopien teilen sich zunächst den
Datenzeiger (`m_pData`) und einen am Ende des Arrays mitgeführten Referenzzähler; erst bei
einer schreibenden Operation (`SetElement`) wird bei `RefCount > 1` eine echte Kopie angelegt.

## 5. Konfigurationsdatei (`NWNArmory.ini` / `standard.ini`)

Beide Dateien sind inhaltlich identisch und enthalten `nTransforms=110`, also 110 Sektionen
`[s0]`–`[s109]`. Jede Sektion definiert eine Regel für **ein Körperteil × eine Rasse**, z. B.:

```ini
[s0]
match=pm??_belt???
substitute=??a*
Scale=(0.72, 0.72, 0.72)
```

Aufbau des Namensschemas (typisch für NWN-Rüstungsteile): `p` (Player) `m`/`f` (Geschlecht)
`??` (Phänotyp-Nummer) `_` `<teil>` `???` (Variante). Das `substitute`-Muster `??a*` behält die
ersten zwei Zeichen (Geschlecht+Phänotyp-Ziffer) und fügt an dritter Stelle den Rassen-Buchstaben
ein (`a`=Halbling, `d`=Zwerg, `e`=Elf, `g`=Gnom, `o`=Halbork), der Rest wird per `*` übernommen.
Alle 10 Körperteile (Gürtel, Brust, Hals, Becken, Schulter, Bizeps, Unterarm, Hand, Bein,
Schienbein, Fuß) existieren jeweils für Männer und Frauen × 5 Rassen = 100 Sektionen, plus 10
weitere (vermutlich die im Diff sichtbaren Erweiterungen) ergeben die 110.

Nicht gesetzte Werte fallen auf sinnvolle Defaults zurück (Identität): Scale `(1,1,1)`,
Rotate/Translate `(0,0,0)`, Min/Max `(-999…, 999…)` (siehe `readme.txt`, Versionshistorie 1.1/1.2).

## 6. Auffälligkeiten, Risiken, technische Schulden

- **O(Dateien × Regeln)-Komplexität ohne Kurzschluss**: Für jede Quelldatei werden alle
  (bis zu 1024) Regeln durchprobiert – bei 110 Regeln und vielen Quelldateien kann das
  spürbar dauern, ist aber unkritisch bei den üblichen Batch-Größen.
- **Keine Kollisionsprüfung**: Wenn zwei Regeln denselben Zielnamen erzeugen, wird die
  vorherige Datei stillschweigend überschrieben (`CFile::modeCreate` ohne Existenzprüfung).
- **`CIO::SetOutFile`**: `CFileException fileException;` wird deklariert, aber im Fehlerfall
  nie durch `CFile::Open` befüllt (der `pError`-Parameter wird nicht übergeben) – die
  `TRACE`-Ausgabe zeigt daher immer eine uninitialisierte Ursache.
- **Kein Zeilen-Zähler bei `ProcessVerts`/`ProcessTverts` bei vorzeitigem EOF im mittleren
  Bereich der Datei ohne weitere Wiederherstellung** – Datei bleibt teilweise geschrieben,
  Exception bricht nur die aktuelle Regel/Datei ab (durch die `catch`-Blöcke in
  `OnBnClickedBtnGo`), die Verarbeitung der übrigen Quelldateien läuft aber weiter.
  Angesichts des internen Kommentars in `NWNArmoryDlg.cpp` ("does not check if it is
  overwriting files", "assumes all models are at position 0 0 0") sind das bekannte,
  dokumentierte Einschränkungen der ursprünglichen Autoren.
- **Rein ASCII-`.mdl`, kein Binärformat**: Laut dem beigelegten `nwn-mdl-format_v5.md`
  unterstützt NWN auch ein binäres `.mdl`-Format; NWNArmory verarbeitet ausschließlich das
  zeilenbasierte ASCII-Format. Modelle, die nur binär vorliegen, müssten vorher konvertiert
  werden (z. B. mit einem externen Tool).
- **`wildcmp`**: eigene, einfache Wildcard-Implementierung (kein Escaping, kein `?`-Fallback
  bei leerem String außerhalb der Schleifenbedingung) – für die im Projekt genutzten,
  einfachen Muster ausreichend.
- **MFC/VC++-Alter**: Projektdateien liegen als `.vcproj`/`.sln` im VS2002/2003-Format vor
  (`Version="7.10"`/`Format Version 8.00`), reine ASCII-`CString`, keine Unicode-Builds.
  Ein Rebuild erfordert eine passende (ältere) MFC-Toolchain oder eine Migration auf ein
  aktuelles Visual-Studio-Projektformat.

## 7. Kurzfassung des Datenflusses

```
Quell-.mdl (ASCII) ──► CIO liest Zeile für Zeile
                         │
                         ├─ Dateiname passt zu match? ──nein──► Regel überspringen
                         │        │ ja
                         │        ▼
                         │  neuer Modellname aus substitute
                         │        │
                         │        ▼
              CTransform liefert vorab berechnete
              Scale-/Rotate-/Translate-Matrizen (Vertex & Textur)
                         │
                         ▼
         verts/tverts/position-Zeilen werden per CMatrix
         transformiert, alle anderen Zeilen kopiert
         (Modellname textuell ersetzt)
                         │
                         ▼
              Ziel-.mdl im Zielverzeichnis
```

## 8. Fazit

NWNArmory ist ein kompaktes, INI-gesteuertes Batch-Transformationswerkzeug: Es kombiniert
einen einfachen Wildcard-Dateinamen-Matcher mit einer klassischen 4×4-Matrix-Transformations-
kette, um aus menschlichen NWN-Rüstungsmodellen automatisiert rassenspezifische Skalierungen
zu erzeugen. Die Geschäftslogik steckt fast vollständig in `Transform`/`Matrix`/`IO`; die
MFC-Dialoge (`NWNArmoryDlg`, `BDialog`, `FolderDialog`, `ProgressWnd`, `ShowTransformsDlg`,
`MyHyperLink`) sind reine Bedienoberfläche darum herum. Die mitgelieferten `NWNArmory.ini`
und `standard.ini` enthalten die eigentliche "Rassentabelle" und sind damit der Teil, den man
am ehesten anpassen oder erweitern möchte (z. B. für zusätzliche Rassen oder Körperteile),
ohne den C++-Code selbst ändern zu müssen.
