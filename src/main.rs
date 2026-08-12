// nwnarmory: CLI rewrite of NWNArmory (see NWNArmory_Project_Analysis_EN.md).
//
// Fixed bugs compared to the original:
//   1. No collision check for target files -> silent overwrite.
//      Fix: `written` HashSet tracks target paths in this run; issues a
//      warning and skips on collision instead of overwriting.
//   2. CFileException never populated (pError parameter missing in Open()).
//      Fix: automatically handled by Rust's Result<T, io::Error>.
//   3. In case of an error mid-processing, a partially written target file
//      was left behind.
//      Fix: Write to `<target>.tmp`, then atomically publish it only when
//      the final target name is still free (never overwrite an existing file).
//
// Deliberately NOT fixed / not ported (YAGNI, see CLAUDE.md /
// Ponytail rules of the accompanying repo):
//   - Only ASCII .mdl is supported, just like in the original.
//   - No GUI. The purpose (select INI, select source files, select target folder,
//     "Go") is mapped 1:1 to CLI arguments.
//
// EE (Enhanced Edition) additions the original tool predates:
//   - `normals N` blocks (explicit per-vertex normals) are now transformed
//     using the inverse-transpose of the Scale/Rotate matrix, not the plain
//     vertex transform -- see Transform::apply_normal for why. Direction
//     vectors, so no Translate and no min/max range gating (see doc comment
//     on apply_normal for the rationale).
//   - `tangents N` blocks (Vec4: xyz direction + w handedness) reuse
//     apply_normal for xyz; w is +-1 and untouched by Scale/Rotate, so it
//     passes through unchanged.
//   - `tverts1`/`tverts2`/`tverts3` (EE lightmap / extra UV channels) reuse
//     the same TScale/TRotate/TTranslate math as `tverts`, but are not
//     gated by `tbitmap` -- see Transform::apply_tvert_extra for why.
//   - `animverts`/`animtverts` (EE animmesh position/UV keyframes) reuse
//     `write_transformed_verts`/`write_transformed_tverts` as-is: same
//     "N vec3 lines" layout as `verts`/`tverts`, no new transform needed.
//   - `materialname`, `weights`, `constraints` still pass through as
//     text-only (unchanged from the model's source values), same as every
//     other unrecognised line. Tracked as follow-up work, not yet handled
//     here.

mod fit;
mod transform;

use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use transform::{
    build_substitute, load_transforms_from_path, wildcard_match, PositionMode, Transform, Vec3,
};

fn main() {
    if let Err(e) = run() {
        eprintln!("nwnarmory: Error: {e}");
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!("Usage: nwnarmory [--debug|-d] [--rename-bitmap[=NAME]] <transforms.ini> <source_file_or_folder> <target_folder>");
    eprintln!("       nwnarmory [--values|-v] <source.mdl> <target.mdl>");
    eprintln!();
    eprintln!("Applies the scaling/rotation/translation rules defined in <transforms.ini>");
    eprintln!("to ASCII NWN .mdl files (race variants).");
    eprintln!("  --debug, -d;               Shows ignored/erroneous lines when loading the INI.");
    eprintln!("  --values, -v;              Fits scale/rotate/translate between two .mdl files and prints INI-ready output.");
    eprintln!(
        "  --rename-bitmap[=NAME];    Off by default: the bitmap/texture line is left untouched."
    );
    eprintln!("                             Bare flag: substitute the model name into it, like the old default.");
    eprintln!("                             With =NAME: set the bitmap line to that literal texture name instead.");
}

/// What to do with a `bitmap <name>` line while rewriting a model.
/// ponytail: default is Keep, matching what modders actually do in practice
/// (see NWNArmory-Analysis.md #6 -- the old tool always renamed, users rarely
/// wanted that). Explicit opt-in restores the old behavior.
enum BitmapMode {
    /// Leave the bitmap line exactly as in the source file.
    Keep,
    /// Substitute the model name into the bitmap line (old default behavior).
    RenameToModel,
    /// Replace the bitmap value with this fixed texture name.
    RenameTo(String),
}

/// Parses `--rename-bitmap` / `--rename-bitmap=NAME` out of the raw args.
/// Returns BitmapMode::Keep if the flag is absent.
fn parse_bitmap_mode(args: &[String]) -> BitmapMode {
    for a in args {
        if let Some(name) = a.strip_prefix("--rename-bitmap=") {
            return BitmapMode::RenameTo(name.to_string());
        }
        if a == "--rename-bitmap" {
            return BitmapMode::RenameToModel;
        }
    }
    BitmapMode::Keep
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();

    if raw_args.iter().any(|a| a == "--values" || a == "-v") {
        let rest: Vec<&String> = raw_args
            .iter()
            .filter(|a| *a != "--values" && *a != "-v")
            .collect();
        if rest.len() != 2 {
            eprintln!("Usage: nwnarmory --values|-v <source.mdl> <target.mdl>");
            std::process::exit(2);
        }
        return fit::run_values(rest[0], rest[1]);
    }

    let debug = raw_args.iter().any(|a| a == "--debug" || a == "-d");
    let bitmap_mode = parse_bitmap_mode(&raw_args);

    let args: Vec<String> = raw_args
        .into_iter()
        .filter(|a| {
            a != "--debug"
                && a != "-d"
                && a != "--rename-bitmap"
                && !a.starts_with("--rename-bitmap=")
        })
        .collect();

    if args.len() != 3 {
        print_usage();
        std::process::exit(2);
    }
    run_transform(
        &args[0],
        &args[1],
        PathBuf::from(&args[2]),
        debug,
        &bitmap_mode,
    )
}

fn run_transform(
    ini_path: &str,
    src_arg: &str,
    dest_dir: PathBuf,
    debug: bool,
    bitmap_mode: &BitmapMode,
) -> Result<(), Box<dyn std::error::Error>> {
    let ini_text = fs::read_to_string(ini_path)
        .map_err(|e| format!("cannot read INI file '{ini_path}': {e}"))?;
    let transforms = load_transforms_from_path(ini_path, &ini_text, debug)?;
    eprintln!("{} transform rules loaded.", transforms.len());

    let src_files = collect_source_files(src_arg)?;
    if src_files.is_empty() {
        eprintln!("No .mdl source files found in '{src_arg}'.");
        return Ok(());
    }
    fs::create_dir_all(&dest_dir)?;

    let mut reserved: HashSet<PathBuf> = HashSet::new();
    let mut processed = 0usize;
    let mut skipped_collisions = 0usize;
    let mut failed = 0usize;

    for src_path in &src_files {
        let stem = src_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();

        let mut matched_any = false;
        for t in &transforms {
            if !wildcard_match(&t.match_pat, &stem) {
                continue;
            }
            matched_any = true;
            let ext = src_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mdl");
            let out_name = build_substitute(&stem, &t.substitute);
            let out_path = dest_dir.join(format!("{out_name}.{ext}"));

            if !reserved.insert(out_path.clone()) {
                eprintln!(
                    "Warning: Target file '{}' is already reserved in this run, skipping (from {}).",
                    out_path.display(),
                    src_path.display()
                );
                skipped_collisions += 1;
                continue;
            }
            if path_exists(&out_path)? {
                eprintln!(
                    "Warning: Target file '{}' already exists, skipping (from {}).",
                    out_path.display(),
                    src_path.display()
                );
                skipped_collisions += 1;
                continue;
            }
            eprintln!(
                "Processing {} -> {}",
                src_path.display(),
                out_path.display()
            );
            if let Err(e) =
                process_model(src_path, &stem, &out_name, &out_path, t, bitmap_mode, debug)
            {
                eprintln!("  Error in {}: {e}", src_path.display());
                failed += 1;
                continue;
            }
            processed += 1;
        }
        if !matched_any {
            eprintln!("Warning: '{}' does not match any transform rule, skipped. Use --debug for details.", src_path.display());
            if debug {
                eprintln!("  Model name (stem): '{stem}'");
                eprintln!(
                    "  Loaded match patterns: {}",
                    transforms
                        .iter()
                        .map(|t| t.match_pat.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }

    eprintln!(
        "Done: {processed} file(s) written, {skipped_collisions} skipped due to name collision, {failed} file(s) failed."
    );
    if skipped_collisions > 0 || failed > 0 {
        return Err(format!(
            "batch completed with {skipped_collisions} collision(s) and {failed} failure(s)"
        )
        .into());
    }
    Ok(())
}

fn path_exists(path: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("cannot inspect target path '{}': {e}", path.display()).into()),
    }
}

fn collect_source_files(src_arg: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let path = Path::new(src_arg);
    if path.is_dir() {
        let mut out = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            if p.extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("mdl"))
                .unwrap_or(false)
            {
                out.push(p);
            }
        }
        out.sort();
        Ok(out)
    } else {
        Ok(vec![path.to_path_buf()])
    }
}

/// Processes a source file against a single transform rule and
/// writes the result atomically (via a .tmp file) to `out_path`.
fn process_model(
    src_path: &Path,
    src_stem: &str,
    dest_stem: &str,
    out_path: &Path,
    t: &Transform,
    bitmap_mode: &BitmapMode,
    debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check again immediately before writing. The caller preflights collisions,
    // but this preserves the no-overwrite guarantee when a target appears while
    // a batch is running.
    if path_exists(out_path)? {
        return Err(format!("target file '{}' already exists", out_path.display()).into());
    }

    let tmp_path = temporary_path(out_path)?;
    let result = process_model_inner(
        src_path,
        src_stem,
        dest_stem,
        &tmp_path,
        t,
        bitmap_mode,
        debug,
    );
    match result {
        Ok(()) => {
            // `rename` overwrites an existing file on Unix. A hard link creates
            // the destination only if it does not exist, so publishing remains
            // atomic and never replaces an existing user file.
            if let Err(e) = fs::hard_link(&tmp_path, out_path) {
                let _ = fs::remove_file(&tmp_path);
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    return Err(
                        format!("target file '{}' already exists", out_path.display()).into(),
                    );
                }
                return Err(format!(
                    "cannot publish '{}' without overwriting an existing file: {e}",
                    out_path.display()
                )
                .into());
            }
            fs::remove_file(&tmp_path)?;
            Ok(())
        }
        Err(e) => {
            // Bugfix compared to original: don't leave a half-finished file behind.
            let _ = fs::remove_file(&tmp_path);
            Err(e)
        }
    }
}

fn temporary_path(out_path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let file_name = out_path
        .file_name()
        .ok_or_else(|| format!("target path '{}' has no file name", out_path.display()))?;
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(".tmp");
    Ok(out_path.with_file_name(temporary_name))
}

fn model_error(path: &Path, line: usize, block: &str, message: &str) -> Box<dyn std::error::Error> {
    format!(
        "{}:{}: Block '{}': {}",
        path.display(),
        line,
        block,
        message
    )
    .into()
}

fn parse_block_count(
    path: &Path,
    line_no: usize,
    block: &str,
    token: Option<&str>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let raw = token.ok_or_else(|| model_error(path, line_no, block, "Quantity missing"))?;

    raw.parse::<usize>()
        .map_err(|_| model_error(path, line_no, block, &format!("invalid quantity '{raw}'")))
}

fn parse_numbers(
    path: &Path,
    line_no: usize,
    block: &str,
    line: &str,
    expected: usize,
) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let values: Result<Vec<f32>, _> = line
        .split_whitespace()
        .map(|token| token.parse::<f32>())
        .collect();

    let values = values
        .map_err(|_| model_error(path, line_no, block, &format!("invalid number in '{line}'")))?;

    if values.len() != expected || values.iter().any(|value| !value.is_finite()) {
        return Err(model_error(
            path,
            line_no,
            block,
            &format!("{} numbers expected, received {}", expected, values.len()),
        ));
    }

    Ok(values)
}

fn next_block_line(
    lines: &mut std::io::Lines<BufReader<fs::File>>,
    line_no: &mut usize,
    path: &Path,
    block: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    *line_no += 1;

    match lines.next() {
        Some(Ok(line)) => Ok(line),
        Some(Err(error)) => Err(model_error(
            path,
            *line_no,
            block,
            &format!("Reading error: {error}"),
        )),
        None => Err(model_error(path, *line_no, block, "unexpected end of file")),
    }
}

fn process_model_inner(
    src_path: &Path,
    src_stem: &str,
    dest_stem: &str,
    tmp_path: &Path,
    t: &Transform,
    bitmap_mode: &BitmapMode,
    debug: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let reader = BufReader::new(fs::File::open(src_path)?);

    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp_path)
        .map_err(|error| {
            format!(
                "cannot create temporary output '{}': {error}",
                tmp_path.display()
            )
        })?;

    let mut last_bitmap = String::new();
    // Only the file's first "position" line (the model's root/pivot node, see
    // README: "identifies the pivot point") may take an absolute override from
    // the INI. Every other node's "position" is a local offset relative to its
    // own parent, not a global coordinate.
    let mut position_seen = false;
    let mut lines = reader.lines();
    let mut line_no = 0usize;

    while let Some(result) = lines.next() {
        line_no += 1;
        let line = result?;

        let trimmed = line.trim_start();
        let mut it = trimmed.split_whitespace();
        let keyword = it.next().unwrap_or("").to_lowercase();

        match keyword.as_str() {
            "verts" | "animverts" | "normals" | "tangents" | "tverts" | "animtverts"
            | "tverts1" | "tverts2" | "tverts3" => {
                let block = keyword.as_str();
                let count = parse_block_count(src_path, line_no, block, it.next())?;

                writeln!(out, "{}", replace_no_case(&line, src_stem, dest_stem))?;

                let extra_uv = matches!(block, "tverts1" | "tverts2" | "tverts3");
                let uv = matches!(
                    block,
                    "tverts" | "animtverts" | "tverts1" | "tverts2" | "tverts3"
                );
                let normal = block == "normals";
                let tangent = block == "tangents";
                let expected_values = if tangent { 4 } else { 3 };

                for _ in 0..count {
                    let item_line = next_block_line(&mut lines, &mut line_no, src_path, block)?;
                    let values =
                        parse_numbers(src_path, line_no, block, &item_line, expected_values)?;

                    if normal {
                        write_vec3(
                            &mut out,
                            t.apply_normal([values[0], values[1], values[2]]),
                            &item_line,
                        )?;
                    } else if tangent {
                        match t.apply_normal([values[0], values[1], values[2]]) {
                            Some(value) => writeln!(
                                out,
                                "    {:.7} {:.7} {:.7} {:.7}",
                                value[0], value[1], value[2], values[3]
                            )?,
                            None => writeln!(out, "{item_line}")?,
                        }
                    } else if uv {
                        let transformed = if extra_uv {
                            t.apply_tvert_extra(values[0], values[1])
                        } else {
                            t.apply_tvert(values[0], values[1], &last_bitmap)
                        };

                        match transformed {
                            Some((x, y)) => {
                                writeln!(out, "    {:.7} {:.7} {:.7}", x, y, values[2])?
                            }
                            None => writeln!(out, "{item_line}")?,
                        }
                    } else {
                        match t.apply_vertex([values[0], values[1], values[2]]) {
                            Some(value) => writeln!(
                                out,
                                "    {:.7} {:.7} {:.7}",
                                value[0], value[1], value[2]
                            )?,
                            None => writeln!(out, "{item_line}")?,
                        }
                    }
                }
            }

            "bitmap" => {
                let name = it
                    .next()
                    .ok_or_else(|| model_error(src_path, line_no, "bitmap", "Key/Value missing"))?;

                last_bitmap = name.to_lowercase();

                let indent = &line[..line.len() - trimmed.len()];
                let out_line = match bitmap_mode {
                    BitmapMode::Keep => line.clone(),
                    BitmapMode::RenameToModel => replace_no_case(&line, src_stem, dest_stem),
                    BitmapMode::RenameTo(name) => format!("{indent}bitmap {name}"),
                };

                writeln!(out, "{out_line}")?;
            }

            "position" => {
                let raw_values = it.collect::<Vec<_>>().join(" ");
                let values = parse_numbers(src_path, line_no, "position", &raw_values, 3)?;

                let parsed = [values[0], values[1], values[2]];

                // Bugfix: PositionMode::Absolute used to overwrite EVERY node's
                // "position" line with the same value, collapsing the entire
                // skeleton onto the root pivot's coordinate. Only the first
                // "position" line in the file is the root pivot; every later
                // one is a child bone's local offset and must be transformed
                // like a vertex regardless of the configured mode.
                let position = if !position_seen {
                    position_seen = true;
                    match t.position {
                        PositionMode::Absolute(position) => position,
                        PositionMode::LikeVertex => t.apply_vertex(parsed).unwrap_or(parsed),
                    }
                } else {
                    if debug && matches!(t.position, PositionMode::Absolute(_)) {
                        eprintln!(
                            "Debug: {}:{}: additional 'position' line after the first one; treating it as a vertex offset, not the absolute pivot override.",
                            src_path.display(),
                            line_no
                        );
                    }
                    t.apply_vertex(parsed).unwrap_or(parsed)
                };

                writeln!(
                    out,
                    "  position {:.7} {:.7} {:.7}",
                    position[0], position[1], position[2]
                )?;
            }

            "filedependancy" | "filedependency" => {
                writeln!(out, "{line}")?;
            }

            "setsupermodel" => {
                // setsupermodel <modelname> <supermodelname>
                // <modelname> is the full source stem and gets swapped like any
                // other line via replace_no_case. <supermodelname> (e.g. "pmh0")
                // is a bare race/phenotype code, never containing the piece
                // suffix, so replace_no_case never matches it and it survives
                // untouched, silently leaving the generated model pointed at
                // the wrong race's supermodel. Apply the same wildcard
                // substitute pattern used for the model name to this token too.
                let model_name = it.next().ok_or_else(|| {
                    model_error(src_path, line_no, "setsupermodel", "model name missing")
                })?;
                let super_name = it.next().ok_or_else(|| {
                    model_error(
                        src_path,
                        line_no,
                        "setsupermodel",
                        "supermodel name missing",
                    )
                })?;

                let new_model = replace_no_case(model_name, src_stem, dest_stem);
                let new_super = build_substitute(super_name, &t.substitute);

                writeln!(out, "setsupermodel {new_model} {new_super}")?;
            }

            _ => {
                writeln!(out, "{}", replace_no_case(&line, src_stem, dest_stem))?;
            }
        }
    }

    out.flush()?;

    out.sync_all()?;

    Ok(())
}

fn write_vec3(
    out: &mut fs::File,
    value: Option<Vec3>,
    original: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match value {
        Some(value) => writeln!(out, "    {:.7} {:.7} {:.7}", value[0], value[1], value[2])?,
        None => writeln!(out, "{original}")?,
    }

    Ok(())
}

/// Case-insensitive replacement of all occurrences of `from` by `to` in `line`
/// (Replacement for ReplaceNoCase in IO.cpp).
fn replace_no_case(line: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return line.to_string();
    }
    let lower_line = line.to_lowercase();
    let lower_from = from.to_lowercase();
    let mut result = String::with_capacity(line.len());
    let mut pos = 0usize;
    while let Some(found) = lower_line[pos..].find(&lower_from) {
        let start = pos + found;
        result.push_str(&line[pos..start]);
        result.push_str(to);
        pos = start + from.len();
    }
    result.push_str(&line[pos..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_no_case_basic() {
        assert_eq!(
            replace_no_case("bitmap PM01_BELT001", "pm01_belt001", "pm01a_belt001"),
            "bitmap pm01a_belt001"
        );
    }
    #[test]
    fn malformed_mdl_header_is_rejected_with_context() {
        let dir =
            std::env::temp_dir().join(format!("nwnarmory-strict-test-{}", std::process::id()));

        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = dir.join("broken.mdl");
        let out = dir.join("broken.tmp");

        fs::write(&src, "newmodel broken\nverts nope\n").unwrap();

        let transform = Transform {
            match_pat: "broken".into(),
            substitute: "broken".into(),
            scale: [1.0; 3],
            rotate_deg: [0.0; 3],
            translate: [0.0; 3],
            min: [-999.0; 3],
            max: [999.0; 3],
            tscale: [1.0; 2],
            trotate_z_deg: 0.0,
            ttranslate: [0.0; 2],
            tmin: [-999.0; 2],
            tmax: [999.0; 2],
            tbitmap: None,
            position: PositionMode::LikeVertex,
        };

        let error = process_model_inner(
            &src,
            "broken",
            "broken",
            &out,
            &transform,
            &BitmapMode::Keep,
            false,
        )
        .expect_err("invalid block count must fail")
        .to_string();

        assert!(
            error.contains("broken.mdl:2")
                && error.contains("Block 'verts'")
                && error.contains("invalid quantity"),
            "{error}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn setsupermodel_rewrites_both_model_and_supermodel_name() {
        let dir =
            std::env::temp_dir().join(format!("nwnarmory-supermodel-test-{}", std::process::id()));

        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = dir.join("pmh0_robe112.mdl");
        let out = dir.join("out.tmp");

        fs::write(
            &src,
            "newmodel pmh0_robe112\nsetsupermodel pmh0_robe112 pmh0\ndonemodel pmh0_robe112\n",
        )
        .unwrap();

        let transform = Transform {
            match_pat: "pm??_robe???".into(),
            substitute: "??a*".into(),
            scale: [1.0; 3],
            rotate_deg: [0.0; 3],
            translate: [0.0; 3],
            min: [-999.0; 3],
            max: [999.0; 3],
            tscale: [1.0; 2],
            trotate_z_deg: 0.0,
            ttranslate: [0.0; 2],
            tmin: [-999.0; 2],
            tmax: [999.0; 2],
            tbitmap: None,
            position: PositionMode::LikeVertex,
        };

        process_model_inner(
            &src,
            "pmh0_robe112",
            "pma0_robe112",
            &out,
            &transform,
            &BitmapMode::Keep,
            false,
        )
        .unwrap();

        let written = fs::read_to_string(&out).unwrap();
        assert!(
            written.contains("setsupermodel pma0_robe112 pma0"),
            "supermodel line not rewritten correctly: {written}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn absolute_position_only_overrides_the_first_node() {
        let dir =
            std::env::temp_dir().join(format!("nwnarmory-position-test-{}", std::process::id()));

        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src = dir.join("pmh0_test.mdl");
        let out = dir.join("out.tmp");

        // Two "position" lines: rootdummy (the pivot -- gets the absolute
        // override) and a child bone (must be transformed like a vertex
        // instead, or every bone in the skeleton would collapse onto the
        // pivot's coordinate).
        fs::write(
            &src,
            "newmodel pmh0_test\n\
             beginmodelgeom pmh0_test\n\
             node dummy rootdummy\n\
             parent NULL\n\
             position 1.0 2.0 3.0\n\
             endnode\n\
             node trimesh child_g\n\
             parent rootdummy\n\
             position 0.5 0.5 0.5\n\
             endnode\n\
             endmodelgeom pmh0_test\n\
             donemodel pmh0_test\n",
        )
        .unwrap();

        let transform = Transform {
            match_pat: "pm??_test".into(),
            substitute: "??a*".into(),
            scale: [2.0, 2.0, 2.0],
            rotate_deg: [0.0; 3],
            translate: [0.0; 3],
            min: [-999.0; 3],
            max: [999.0; 3],
            tscale: [1.0; 2],
            trotate_z_deg: 0.0,
            ttranslate: [0.0; 2],
            tmin: [-999.0; 2],
            tmax: [999.0; 2],
            tbitmap: None,
            position: PositionMode::Absolute([9.0, 9.0, 9.0]),
        };

        process_model_inner(
            &src,
            "pmh0_test",
            "pma0_test",
            &out,
            &transform,
            &BitmapMode::Keep,
            false,
        )
        .unwrap();

        let written = fs::read_to_string(&out).unwrap();
        let positions: Vec<&str> = written
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("position "))
            .collect();

        assert_eq!(positions.len(), 2, "expected 2 position lines: {written}");
        assert_eq!(
            positions[0], "position 9.0000000 9.0000000 9.0000000",
            "first position (root pivot) must take the absolute override: {written}"
        );
        assert_eq!(
            positions[1], "position 1.0000000 1.0000000 1.0000000",
            "second position (child bone) must be scaled like a vertex, not overwritten: {written}"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
