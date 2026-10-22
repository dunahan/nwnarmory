# NWNArmory Rust Port — Original & EE Format Coverage

Status snapshot as of **v1.3.5**.
Scope: which ASCII `.mdl` fields the Rust CLI (`src/main.rs`/`src/transform.rs`) reads, transforms, or passes through, checked against (a) what the original C++ tool (`IO.cpp`/`Transform.cpp`, see `docs/NWNArmory-Analysis.md`) did, and (b) the full field list in `nwn-mdl-format_v5.md`. Binary `.mdl`, `.mtr`, and `.txi` files are unchanged: out of scope for both tools (ASCII `.mdl` only, one file per run).

## Part 1 — Original (pre-EE, "1.69") tool: full parity, plus two fixes beyond it

Every field the original tool special-cased is special-cased identically in the Rust port:

| Field | Original behavior | Rust port |
|---|---|---|
| `verts` | Scale → Rotate → Translate per vertex, only if in `minimum`/`maximum` range, else copied | Identical (`apply_vertex`) |
| `tverts` | TScale → TRotate(Z) → TTranslate, gated by `tbitmap` + `tminimum`/`tmaximum` | Identical (`apply_tvert`) |
| `bitmap` | Always renamed to the new model name | **Diverges on purpose**: default is now *Keep* (see `CHANGELOG` 1.2.4) — the old always-rename behavior is opt-in via `--rename-bitmap` |
| `position` | `position=` is an absolute root-pivot override; child positions are local offsets | First position gets the absolute override; child offsets get scale/rotation only, without translation accumulating down the hierarchy |
| `filedependancy`/`filedependency` | Copied verbatim | Identical |
| Everything else | Generic model-name text substitution | Identical (`replace_no_case`) |

**Two deliberate improvements over the original, not parity gaps:**
- **`position=` no longer corrupts multi-node parts.** The original's matrix-multiplication trick (a translation-only matrix with a zeroed diagonal) makes *every* `position` line collapse onto the configured absolute value once one is set — harmless for a single-pivot piece (the common case docs describe), destructive for any part with a second `position` line (e.g. a child bone). The Rust port only overrides the first.
- **`setsupermodel` now targets the correct race.** The original's plain text substitution only matches the full model stem, so the bare supermodel token (e.g. `pmh0`, without the piece suffix) was left pointing at the source race. Fixed in v1.3.3.

**Conclusion: goal (a) is done.** Nothing from the original tool's documented behavior is missing, and the two known original-tool bugs above are fixed rather than reproduced.

## Part 2 — NWN:EE format support

### Implemented

| Field(s) | Handling |
|---|---|
| `normals` | Inverse-transpose of Scale/Rotate + renormalize (`apply_normal`) — correct under non-uniform scale, tested (`transform::tests::normal_*`) |
| `tangents` | Same inverse-transpose for the xyz part; the `w` handedness value (`±1`) is passed through unchanged, never renormalized |
| `tverts1`/`tverts2`/`tverts3` (extra UV channels) | Same TScale/TRotate/TTranslate as `tverts`, but **not** gated by `tbitmap` (that option predates multi-channel UVs) — tested (`tvert_extra_ignores_tbitmap_filter`) |
| `animverts`/`animtverts` | Read/written as plain `N × Vector3` blocks, same as `verts`/`tverts` |
| `materialname` (`.mtr` reference) | Own `Keep`/rename mode via `--rename-materialname[=NAME]`, mirroring `bitmap`'s policy and default |
| `weights` (skin bone/weight pairs), `constraints` (danglymesh rigidity) | Consumed as counted blocks (truncation-safe, matching `verts`/`tverts`); never transformed — neither is a scale-dependent distance |
| `colors` (per-vertex custom colour) | Consumed as a counted `N × Vector3` block. Entries must contain exactly three finite numbers, but valid source text is written back verbatim because colour is not geometry. |
| Counted block entries | Truly empty lines are skipped while collecting the declared entries; a truncated block still returns a contextual error instead of publishing partial output. |

The current regression suite also exercises a complete static trimesh with all four UV channels, normals,
tangents, colors, faces, and mesh metadata; checks extra UV channels when `tbitmap` is set; verifies that
only the root position takes an absolute override; and covers default/explicit `materialname` behavior and
verbatim `filedependancy`/`filedependency` references during model renaming.

### Known gaps, ranked by likely impact on real armor pieces

1. **Animated keyframe controllers are never transformed or block-consumed**, on any node type: `positionkey`, `orientationkey`, `scalekey`, and the animated float keys on `Light`/`Emitter` (`colorkey`, `radiuskey`, `alphaStart`/etc.). If an armor piece has an *animated* position or scale (rare for standard belt/chest/neck/… pieces; more plausible for a custom cape/hair with a keyframed sway, or an item with an attached emitter), the keyframes keep the source race's proportions after a resize, and a truncated keyed block currently fails silently rather than erroring. This is the largest remaining gap in scope terms, but likely the *least* frequently hit in practice for the tool's actual use case (static armor geometry, not animated creature/cloth rigs).
2. **`--rename-materialname` doesn't touch the referenced `.mtr` file.** Renaming the `materialname` line to the new model's name makes it point at a `.mtr` file that doesn't exist unless the user creates/copies one by hand. This mirrors `--rename-bitmap`'s behavior; the README calls out that both flags only change the reference stored in the `.mdl`.

### Explicitly out of scope (matches the original tool's scope, not a gap)

- **Binary `.mdl`, `.mtr`, `.txi`** — ASCII `.mdl` only, same as the original; convert binary models first (README already documents this).
- **AABB/walkmesh fields** (`aabb` tree, `multimaterial`) and **Emitter distance params** (`deadspace`, `blastRadius`, `blastLength`) are not scaled — same as the original tool. Walkmesh resizing was never NWNArmory's use case (tile geometry, not armor); emitter distances are a niche edge case (particle-emitting items) with no original-tool precedent either.

## Methodology

Compared against `IO.cpp`/`Transform.cpp` (original source), the current `src/main.rs`/`src/transform.rs`, and the ASCII fields listed in `nwn-mdl-format_v5.md`. Test references describe regression coverage in the repository; this document does not claim a fresh test run for this snapshot.
