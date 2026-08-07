# NWNArmory

**Current release: 1.3.2**

**A Neverwinter Nights Model Rescaling CLI Tool**

NWNArmory is a command-line interface (CLI) tool designed to automatically resize and transform standard Neverwinter Nights (NWN) 3D ASCII `.mdl` models into variants suitable for other playable races (Halfling, Dwarf, Elf, Gnome, Half-orc). 

---

## 🌟 Acknowledgements & Original Developers

**This project is a modern rewrite of the original [NWNArmory](https://neverwintervault.org/project/nwn1/other/tool/nwnarmory-12-source-code).** It is of utmost importance to highlight and credit the original creators who made this tool possible. The version of this repository used as a base was provided by the Bloodspell Machinima Film Project.

*   **Original Author:** Eligio Sacateca (Rumbles Designs), June 2003.
*   **Data Compilation:** Based on original data compiled by **Bearthing**.

The original NWNArmory was a Windows MFC/Visual C++ GUI application. You can still find references to the original logic and instructions in the historical `readme.txt` included in the project history.

## 🚀 The Transition to Rust and CLI

The original C++ codebase has been completely rewritten in **Rust**. This transition brings several major structural changes:

*   **Command-Line Interface (CLI):** The GUI has been completely removed. NWNArmory is now a pure CLI tool, making it significantly easier to integrate into automated build pipelines and batch scripts.
*   **Modern Safety & Performance:** Leveraging Rust's ownership and error-handling models guarantees memory safety and predictable execution without legacy C++ runtime issues.

## ✨ Safety & Reliability

Building upon the original v1.2 logic (which introduced absolute translation, minimum/maximum parameters, and texture map scaling), the Rust CLI focuses on deterministic transformations and safe batch processing:

*   **High-Precision Vertex & Node Transformations:** Transformation matrices (scale, rotation, and absolute translation/position) are calibrated for reliable alignment of body parts across race variants.
*   **Normals-aware processing:** Explicit `normals` blocks are transformed using the inverse-transpose of the scale/rotation matrix and normalized, which preserves correct lighting under non-uniform scale.
*   **Safe File Writing:** Output is first written to an exclusively created `.tmp` file. The temporary file is removed when model processing fails, so a truncated model never becomes a target file.
*   **No accidental overwrite:** An existing target file is preserved. If a source/rule pair would produce an existing target name, or if two results in the same run would use the same name, the affected result is skipped and reported.
*   **Automation-safe results:** The tool continues with other eligible models after a collision or model-processing error, but exits with a non-zero status when the batch was not completely successful.
*   **Eliminated Legacy IO Errors:** The notorious `CFileException` unpopulated errors from the original codebase are eliminated through Rust's `Result<T, io::Error>` handling.

## 🛠️ Usage

Installation is simple. You can download the latest [release](https://github.com/dunahan/nwnarmory/releases/latest) or compile it yourself, afterwards you can run the binary directly via your terminal.

- Compile it yourself (Rust must be installed):
```bash
git clone https://github.com/dunahan/nwnarmory.git
cd nwnarmory
cargo build --release
```
You'll find the binaries under `target/release/`.

- Execution of nwnarmory:
```bash
nwnarmory <transforms.ini> <source_file_or_folder> <target_folder>

nwnarmory NWNArmoryv121.ini pmh0_chest001.mdl ./created
```

*   `<transforms.ini>`: The configuration file containing the scaling and translation matrices (e.g., `standard.ini`).
*   `<source_file_or_folder>`: A single `.mdl` file or a directory containing multiple `.mdl` files.
*   `<target_folder>`: The directory where the transformed models will be saved.
*   `--debug` / `-d`: Prints ignored/erroneous INI lines and diagnostics for models that matched no transform rule.
*   `--rename-bitmap[=NAME]`: See "Texture (Bitmap) Renaming" below.

- Deriving transform values from two already-fitted models:
```bash
nwnarmory --values pmh0_chest001.mdl pfa0_chest001.mdl
nwnarmory -v pmh0_chest001.mdl pfa0_chest001.mdl
```

See "Deriving Transform Values" below.

### Output Collision Policy and Exit Status

NWNArmory never replaces a file that already exists in the target directory. It also prevents two source/rule combinations in the same batch from writing to the same target name. In both cases, the affected output is skipped and reported on standard error.

The tool continues processing other eligible models so that a batch produces all safe outputs it can. Its exit status is intended for scripts and CI:

* `0` — all matched models were written successfully and no target collision occurred.
* `1` — one or more model writes failed, or one or more output-name collisions were skipped.
* `2` — command-line usage was invalid.

Do not rely on a zero exit status merely because some output files were created; a partial batch deliberately returns `1`.

## 🧮 Deriving Transform Values (`--values` / `-v`)

If you already have a hand-fitted source and target model (e.g. exported from Max/Blender or 1:1-corresponding race variants) and want the matching `transforms.ini` section instead of eyeballing the numbers, run:

```bash
nwnarmory --values pmh0_chest001.mdl pfa0_chest001.mdl
```

This fits `scale`, `rotate`, `translate` (least-squares over the `verts` block), `tscale`/`trotate`/`ttranslate` (over `tverts`), and `position` (absolute, read directly from the target file) between the two models, and prints the result as INI lines ready to paste into an `[sN]` section. It also derives wildcard-compatible `match=` and `substitute=` lines from the two model names, for example `pm??_chest???` and `??a*`, matching the naming style used by the standard INI. Each fit is followed by a `max_residual`/`mean_residual` line — a high residual means the two files don't relate by a simple scale+rotate+translate (or aren't a matching pair).

**Important:** Check the generated `match=` and `substitute=` lines before using them. They are inferred from only two filenames and may intentionally match more models than the two input files. Verify especially the fixed prefix, the wildcard positions, the target race/model letter, and the generated target name; adjust the patterns manually when your naming scheme differs from the standard NWNArmory convention.

**Note:** this requires vertex correspondence — both models must have the same vertex count and order, i.e. the target was never remeshed relative to the source. This holds for genuine NWNArmory-style race variants.

## 🖼️ Texture (Bitmap) Renaming

By default, the `bitmap` line in generated models is left exactly as in the source file. The very old original tool always tried to rename it to match the new model name; in practice this was rarely what modders wanted, so it's now opt-in:

*   `--rename-bitmap`: substitutes the newly generated model name into the `bitmap` line (the historic default behavior).
*   `--rename-bitmap=NAME`: sets the `bitmap` line to the literal texture name `NAME`, regardless of the generated model name.

Omit the flag entirely if your texture names are independent of the model name (the common case).

## ⚠️ Important Prerequisites

*   **ASCII Format Required:** NWNArmory strictly processes ASCII model files. If your base models are in binary format, you must decompile them into ASCII format (for example, using a tool like [CleanmodelsEE](https://github.com/plenarius/cleanmodels/tree/v4-go-rewrite) or NWNMdlComp) before running them through this utility.

## ⚙️ The INI File Format & Parameters

The transform initialization (`.ini`) file consists of a series of transform groups. Each group requires a match string, a substitute string, and the specific transform definitions. 

### Wildcards (Globbing)
When defining `match=` and `substitute=` strings, you can use wildcards:
*   `?` acts as a wildcard matching any single character.
*   `*` acts as a full glob wildcard matching any number of characters.

### Core Parameters
*   `match=`: The input source model name must match this string for the group to be applied.
*   `substitute=`: Defines how the new file and model name will be generated based on the input name.
*   `scale=(x, y, z)`: Applies x, y, and z-axis scaling.
*   `rotate=(x, y, z)`: Applies x, y, and z-axis rotation.
*   `translate=(x, y, z)`: Applies absolute x, y, and z-axis translation.
*   `position=(x, y, z)`: Specifies the absolute position parameter (which identifies the pivot point for armor parts).
*   `minimum=(x, y, z)` & `maximum=(x, y, z)`: If a vertex falls outside these minimum/maximum coordinate bounds, it will not be transformed.

### Texture Map Parameters
*   `tscale=(x, y)`: Applies x and y-axis scaling to a texture map.
*   `trotate=(z)`: Applies z-axis rotation to the texture map.
*   `ttranslate=(x, y)`: Translates the texture map along the x and y axes.
*   `tminimum=(x, y)` & `tmaximum=(x, y)`: Texture vertices outside these bounds remain untransformed.
*   `tbitmap=<bitmap name>`: Restricts the transformations exclusively to texture maps using this specific bitmap.

## 📐 Technical Notes & Defaults

*   **Order of Operations:** The transformations are strictly applied in the following sequence: scaling, followed by rotation, followed by translation.
*   **Precision & Normals Verification:** Transformation calculations maintain floating-point scientific precision. Output tests demonstrate almost 100% exact alignment across `normals` data blocks and face mappings compared against canonical model outputs.
*   **Origin of Rotation:** Rotations take place exclusively about the origin `(0,0,0)`.
*   **Default Values:** If a parameter is omitted from a transform group, the tool applies default values resulting in no changes. For example, `scale` defaults to `(1, 1, 1)`, `rotate` and `translate` default to `(0, 0, 0)`. The bounds for `minimum` and `maximum` default to `(-999, -999, -999)` and `(999, 999, 999)` respectively, ensuring all vertices are included by default.

## 📄 License

This project is licensed under the GNU Affero General Public License v3.0 (AGPL-3.0).
