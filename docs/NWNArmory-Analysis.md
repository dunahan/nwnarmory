# NWNArmory – Project Analysis

## 1. What the Program Does

NWNArmory is a Windows desktop tool (MFC/Visual C++, apparently from the NWN modding era around 2003) for the **automated resizing of Neverwinter Nights models (`.mdl`, ASCII format) to other playable races**. Typically, one has the standard models for a human character (e.g., armor parts: belt, chest, neck, pelvis, shoulder, bicep, forearm, hand, leg, shin, foot) and wants to automatically generate fitting variants for Halfling, Dwarf, Elf, Gnome, and Half-Orc—without having to scale each part manually in 3ds Max.

The tool reads the source models in text format (ASCII `.mdl`), applies strictly defined scaling, rotation, and translation matrices to the vertex and texture coordinates per body part and race, and writes the result to a target directory under a new filename (race suffix).

## 2. Architecture Overview

```text
NWNArmory.cpp/.h        → CWinApp entry point, launches the main dialog
NWNArmoryDlg.cpp/.h     → Main dialog: file selection, target folder, INI file, "Go" button
BDialog.cpp/.h          → Base dialog class with background image (Tile/Center/Stretch)
FolderDialog.cpp/.h     → Folder selection dialog (SHBrowseForFolder wrapper)
MyHyperLink.cpp/.h      → Hyperlink control (only used in the About dialog)
ProgressWnd.cpp/.h      → Popup progress window during processing
ShowTransformsDlg.cpp/.h→ Lists all loaded transform groups for inspection

NWNGlobals.cpp/.h       → Global transform array, logging (CLogFile), path helper functions
Transform.cpp/.h        → CTransform: one "rule" (match pattern + matrices) per INI section
Matrix.cpp/.h           → CMatrix: generic, reference-counted 4x4/NxM matrix class
IO.cpp/.h               → CIO: reads a source model, checks for rule matches, writes target model
ini2.cpp/.h             → CIni: thin wrapper around the Windows INI API

NWNArmory.ini/standard.ini → Configuration data: 110 transform sections (Race × Body Part)
```

## 3. Execution – Step by Step

1. **Startup (`CNWNArmoryApp::InitInstance`)**: Opens `CNWNArmoryDlg` modally.
2. **`OnInitDialog`**: Automatically loads the transforms from the standard INI (`LoadTransforms()`) upon startup, sets the background image, and disables the "Go" button.
3. **Step 1 – Select Transformation File** (`OnBnClickedBtnTransformation`): The user can select a different `.ini`; `LoadTransforms(path)` reads it anew.
4. **`LoadTransforms`**:
   - Reads `[Global] nTransforms` (number of sections, capped at `MAXTEMPLATES = 1024`).
   - Allocates `CTransform transform[nTransforms]` (previous array is deleted via `delete[]`).
   - For each section `s0..sN-1`: reads `match`, `substitute`, `scale`, `rotate`, `translate`, `minimum`, `maximum` as well as the texture variants (`tscale`, `trotate`, `ttranslate`, `tminimum`, `tmaximum`, `tbitmap`) and `position`. On parsing errors (`sscanf` count mismatch), a message is displayed, and the section is effectively disabled by clearing `match` (`continue`).
   - Opens/creates the log file (`NWNArmory.Log`) as needed.
5. **Step 2 – Select Source Files** (`OnBnClickedBtnSource`): Multi-selection of `.mdl` files via a 32 KB buffer (`filebuf[32768]`); filenames are added to `m_SourceFiles` and the list box. The source directory is derived from the first file.
6. **Step 3 – Select Destination Directory** (`OnBnClickedBtnDestination`): `CFolderDialog`.
7. **Step 4 – "Go!"** (`OnBnClickedBtnGo` → `ProcessSourceFile`):
   For **every** source file, **every** loaded transform rule is evaluated:
   - `CIO io(file, target_directory)` opens the source file for reading and extracts the base name (converted to lowercase) as `mstrSrcModelName`.
   - `io.SetOutFile(transform[i])`:
     - `doesMatch()` checks the filename against `match` using a **wildcard comparison** (`wildcmp`, supports `*` and `?`, custom implementation in `IO.cpp`).
     - On a match: `doSubstitute()` constructs the new model name from `substitute` (also using `?`/`*` placeholders), e.g., `pm01_belt001` + pattern `??a*` → `pm01a_belt001` (the first two characters are kept, the third is replaced by `a` = Halfling suffix).
     - The target file is created (`CFile::modeCreate|modeWrite`), throwing an exception on error.
   - `io.ProcessModel(transform[i])` reads the source file **line by line** and writes the transformed data to the target file (see Section 4).
   - A source model can thus be processed **multiple times**—once for each matching rule (e.g., the same belt file generates Halfling, Dwarf, Elf, Gnome, and Half-Orc variants because five sections exist with the same `match` pattern but different `substitute` strings).
   - A `CProgressWnd` displays progress and allows cancellation; results/errors are logged and partially reported via `AfxMessageBox`.

## 4. Core Logic of Model Transformation (`CIO::ProcessModel`)

The ASCII `.mdl` format is interpreted line by line via token recognition (first word, lowercase):

| Line Type | Behavior |
| :--- | :--- |
| `verts N` | Reads the following `N` vertex lines (`ProcessVerts`/`ProcessVert`), applies `CMatrix` multiplications for scaling, rotation, and translation per vertex (only if the point is within `minimum`/`maximum`, otherwise copied unchanged). |
| `tverts N` | Analogous for texture coordinates, with its own scale/rotation/translation (`ProcessTverts`/`ProcessTvert`); can optionally be restricted to a specific `bitmap` (`tbitmap`). |
| `bitmap <name>` | Remembers the last seen bitmap name (for the `tbitmap` check) and replaces model name references. |
| `position <x y z>` | Transforms the pivot point (`ProcessPosition`), either using the same Scale/Rotate/Translate matrices or as an absolute displacement (`isMovePos`, if `position=(x,y,z)` is set in the INI). |
| `filedependancy` | Copied unchanged (Max file reference, irrelevant to the loader). |
| Everything else | Model name is replaced via `ReplaceNoCase`, line copied unchanged. |

The actual vector transformation utilizes homogeneous 4×1 vectors (`CMatrix v(4,1)`) and 4×4 matrices (scaling, rotation converted from degrees in `CTransform::SetRotateFromDegrees`, translation), which are precalculated once in `CTransform` when the INI is loaded (`m_rotTransform`, etc.). Rotation is explicitly negated for a **left-handed coordinate system** (3ds Max) (comment in `Transform.cpp`: "Max uses a LH-coordinate system").

`CMatrix` is a classic copy-on-write matrix class: copies initially share the data pointer (`m_pData`) and a reference counter kept at the end of the array; an actual copy is only created upon a write operation (`SetElement`) if `RefCount > 1`.

## 5. Configuration File (`NWNArmory.ini` / `standard.ini`)

Both files are identical in content and contain `nTransforms=110`, meaning 110 sections `[s0]`–`[s109]`. Each section defines a rule for **one body part × one race**, e.g.:

```ini
[s0]
match=pm??_belt???
substitute=??a*
Scale=(0.72, 0.72, 0.72)
```

Structure of the naming convention (typical for NWN armor parts): `p` (Player) `m`/`f` (Gender) `??` (Phenotype Number) `_` `<part>` `???` (Variant). The `substitute` pattern `??a*` retains the first two characters (gender + phenotype digit) and inserts the race letter at the third position (`a`=Halfling, `d`=Dwarf, `e`=Elf, `g`=Gnome, `o`=Half-Orc), while the rest is carried over via `*`.
All 10 body parts (belt, chest, neck, pelvis, shoulder, bicep, forearm, hand, leg, shin, foot) exist for men and women × 5 races = 100 sections, plus 10 additional ones yielding the 110.

Unset values fall back to sensible defaults (Identity): Scale `(1,1,1)`, Rotate/Translate `(0,0,0)`, Min/Max `(-999…, 999…)`.

## 6. Anomalies, Risks, and Technical Debt

- **O(Files × Rules) complexity without short-circuiting**: All (up to 1024) rules are evaluated for every source file—with 110 rules and many source files, this can take a noticeable amount of time, but is uncritical for typical batch sizes.
- **No collision detection**: If two rules generate the same target name, the previous file is silently overwritten (`CFile::modeCreate` without existence check).
- **`CIO::SetOutFile`**: `CFileException fileException;` is declared, but never populated by `CFile::Open` in case of an error (the `pError` parameter is not passed)—the `TRACE` output therefore always shows an uninitialized cause.
- **No line counter for `ProcessVerts`/`ProcessTverts` on premature EOF** in the middle of the file without further recovery—the file remains partially written, the exception only aborts the current rule/file, but processing of the remaining source files continues. Given the internal comment in `NWNArmoryDlg.cpp` ("does not check if it is overwriting files", "assumes all models are at position 0 0 0"), these are known, documented limitations from the original authors.
- **Strictly ASCII `.mdl`, no binary format**: NWN also supports a binary `.mdl` format; NWNArmory exclusively processes the line-based ASCII format. Models that are only available as binaries would need to be converted beforehand.
- **`wildcmp`**: Custom, simple wildcard implementation (no escaping, no `?` fallback on an empty string outside the loop condition)—sufficient for the simple patterns used in the project.
- **MFC/VC++ Age**: Project files are in `.vcproj`/`.sln` VS2002/2003 format, pure ASCII `CString`, no Unicode builds. A rebuild requires an appropriate older MFC toolchain or a migration to a current project format.

## 7. Data Flow Summary

```text
Source .mdl (ASCII) ──► CIO reads line by line
                          │
                          ├─ Filename matches 'match'? ──no──► Skip rule
                          │        │ yes
                          │        ▼
                          │  New model name from 'substitute'
                          │        │
                          │        ▼
             CTransform provides precalculated
             Scale/Rotate/Translate matrices (Vertex & Texture)
                          │
                          ▼
        verts/tverts/position lines are transformed via CMatrix,
        all other lines are copied
        (Model name textually replaced)
                          │
                          ▼
             Target .mdl in the destination directory
```

## 8. Conclusion

NWNArmory is a compact, INI-driven batch transformation tool: It combines a simple wildcard filename matcher with a classic 4×4 matrix transformation chain to automatically generate race-specific scalings from human NWN armor models. The business logic is almost entirely contained in `Transform`/`Matrix`/`IO`; the MFC dialogs are purely the user interface wrapped around it. The included `NWNArmory.ini` and `standard.ini` contain the actual "race table" and are thus the part most likely to be adapted or expanded without having to modify the C++ code itself.

## 9. Path to Modernization

Given the technical debt of the legacy MFC framework, modernizing NWNArmory into a Rust-based command-line interface (CLI) provides a robust upgrade path. Transitioning to a Rust CLI allows the tool to run natively across platforms, enabling users to execute transformations effortlessly from a Bash terminal on systems like Linux Mint. The core engine can continue to scale and transform 3D game models efficiently using the `.ini` configuration files. When implementing the Rust parser, strict adherence to the formatting of the existing `.ini` style must be maintained so the program correctly interprets the rulesets. Finally, the transformed ASCII `.mdl` files can be easily previewed and validated by dropping them into modern browser-based 3D model viewers.
