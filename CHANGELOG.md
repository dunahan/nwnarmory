# Changelog

All notable changes to this project will be documented in this file.

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
