# Changelog

All notable changes to this project will be documented in this file.

## [1.3.3] - 2026-08-11

### Fixed
- **Absolute `position=` corrupted every child node's skeleton offset:** When a transform rule set an explicit `position=(x,y,z)` (as `--values` always does when a real pivot value is present), the tool overwrote *every* `position` line found while parsing the model with that single value, collapsing the entire node hierarchy onto the root pivot's coordinate instead of only repositioning the root. Only the file's first `position` line (the model's root/pivot node) now takes the absolute override; every other node's position is transformed like a vertex (scale/rotate/translate), exactly as it already was for the default non-absolute case. `--debug` now warns when an additional `position` line is found while an absolute override is configured.
- **`setsupermodel` line kept pointing at the source race:** The generic model-name text substitution only matches the full source stem (e.g. `pmh0_robe112`), so the bare supermodel token on the same line (e.g. `pmh0`, without the piece suffix) was never replaced, and generated race variants kept referencing the human supermodel instead of their own. `setsupermodel` now rewrites that token too, using the same wildcard `substitute` pattern applied to the model name.

## [1.3.2] - 2026-08-07

### Added
- **Wildcard model-name mapping in `--values` / `-v`:** The fit mode now derives and prints `match=` and `substitute=` lines from the source and target model names, following the wildcard style used by the standard NWNArmory INI.

### Documentation
- **Verification note for generated mappings:** The README now explains that inferred wildcard patterns must be checked before use because they can match additional models beyond the two input files.

## [1.3.1] - 2026-08-07

### Added
- **CLI regression tests:** End-to-end coverage for preserving existing outputs, detecting duplicate target names, cleaning up failed writes, and reporting partial batch failures.

### Changed
- **Safe batch-output policy:** Existing target files are never overwritten. A target-name collision with an existing file or another result in the same batch is reported and skipped.
- **Reliable automation status:** A batch that contains a collision or a model-processing failure now exits with a non-zero status after processing all remaining eligible models.
- **Safe publishing:** Models are written to an exclusively created temporary file and published only if the target path is still free; incomplete temporary files are removed on failure.

## [1.2.4]

### Added
- **`--rename-bitmap[=NAME]` flag:** New opt-in control over the `bitmap` texture line. Bare flag restores the historic behavior (new model name substituted in); `--rename-bitmap=NAME` sets the `bitmap` line to a literal texture name instead.
- **`--values` / `-v` mode:** `nwnarmory --values <source.mdl> <target.mdl>` fits `scale`/`rotate`/`translate`, `tscale`/`trotate`/`ttranslate`, and `position` between two already-authored models via least squares, and prints the result as ready-to-paste `transforms.ini` lines with fit residuals.

### Changed
- **Behavior:** the `bitmap` line is now left unchanged by default. Previously it was always rewritten to the newly generated model name, which in practice was rarely what modders wanted (see `docs/NWNArmory-Analysis.md` #6). Pass `--rename-bitmap` to get the old behavior back.

## [1.2.3]
- Documents, comments, and feedback translated into English.

## [1.2.2]

### Added
- **Debug mode:** Added a new command-line flag `--debug` (or `--d`) for extended diagnostic and error output.
- **Warning for unused models:** The tool now issues a warning if a `.mdl` source file does not match any of the loaded transform rules (`match`). In debug mode, the affected model stem and all checked patterns are also listed.
- **Enhanced INI validation (syntax):** In debug mode, a warning is now issued when malformed lines in the INI file are ignored (e.g., due to a missing `=` sign).
- **Enhanced INI validation (unknown keys):** A warning is now issued if a transform section contains unknown parameters (keywords), making it easier to spot typos. In debug mode, the affected parameters are explicitly listed.

## [1.2.1]

| Original Bug | Fix | Verified by |
| :--- | :--- | :--- |
| Silent overwrite on name collision | `HashSet<PathBuf>` tracks targets per run; warning + skip | Test with pm01_belt001.mdl / PM01_BELT001.mdl → second source correctly skipped |
| `CFileException` never populated | Eliminated by `Result<T, io::Error>` | Structurally guaranteed by Rust |
| Partially written target file on error mid-`verts` block | Write to `.tmp`, atomic rename only upon success | Test with `verts 5` but only 2 lines → target folder remains empty, no `.tmp` residue |

## [1.2]
- Now accepts equivalent functions for transforming texture maps.
- Fixed a couple of annoying bugs with the file selection dialog (now takes up to 64K in files and should not crash if you cancel out of a selection dialog).
- Fixed a couple of rare bugs in the transforms for vertices.

## [1.1]
- Now accepts a "position 0 0 0" line in the source model without complaint. It will apply the standard transforms to this line just like it will to a vertex in the geometry.
- Now accepts a "position=(x, y, z)" transform to move the position vertex if required. This is an absolute translation (it is not a shift relative to its old position). 
- Now accepts a "minimum=(x, y, z)" and a "maximum=(x, y, z)" transform. If the vertex being transformed does not fall into this range, it will not be transformed.
- Now accepts a "translate=(x,y,z)" transform which will shift all points by the specified values.

### Notes on Backward Compatibility
You do not have to change your old .ini files to accommodate the new parameters because they will default if not filled in:
- The minimum and maximimum parameters default to (-999,-999,-999) and (999,999,999) respectively so they will transform all vertices automatically.
- The translate parameter defaults to (0, 0, 0) so it does not move the vertices.
- The position parameter defaults to not change the position.
