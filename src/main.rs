// nwnarmory: CLI-Rewrite von NWNArmory (siehe NWNArmory-Analyse.md).
//
// Behobene Bugs gegenueber dem Original:
//   1. Keine Kollisionspruefung bei Zieldateien -> stilles Ueberschreiben.
//      Fix: `written` HashSet trackt Zielpfade in diesem Lauf, bei
//      Kollision Warnung + Ueberspringen statt Ueberschreiben.
//   2. CFileException nie befuellt (pError-Parameter fehlte in Open()).
//      Fix: entfaellt automatisch durch Rusts Result<T, io::Error>.
//   3. Bei Fehler mitten in der Verarbeitung blieb eine teilweise
//      geschriebene Zieldatei liegen.
//      Fix: Schreiben in `<ziel>.tmp`, erst bei Erfolg atomar umbenennen
//      (rename ist auf demselben Dateisystem atomar).
//
// Bewusst NICHT behoben / nicht uebernommen (YAGNI, siehe CLAUDE.md /
// Ponytail-Regeln des begleitenden Repos):
//   - Nur ASCII-.mdl wird unterstuetzt, wie im Original.
//   - Keine GUI. Der Zweck (INI waehlen, Quelldateien waehlen, Zielordner
//     waehlen, "Go") wird 1:1 durch CLI-Argumente abgedeckt.

mod transform;

use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use transform::{build_substitute, load_transforms, wildcard_match, PositionMode, Transform};

fn main() {
    if let Err(e) = run() {
        eprintln!("nwnarmory: Fehler: {e}");
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!("Verwendung: nwnarmory [--debug|--d] <transforms.ini> <quelldatei_oder_ordner> <zielordner>");
    eprintln!();
    eprintln!("Wendet die in <transforms.ini> definierten Skalierungs-/Rotations-/");
    eprintln!("Translations-Regeln auf ASCII-NWN-.mdl-Dateien an (Rassen-Varianten).");
    eprintln!("  --debug, --d;  Zeigt beim Laden der INI ignorierte/fehlerhafte Zeilen an.");
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let debug = raw_args.iter().any(|a| a == "--debug" || a == "--d");
    let args: Vec<String> = raw_args.into_iter().filter(|a| a != "--debug" && a != "--d").collect();

    if args.len() != 3 {
        print_usage();
        std::process::exit(2);
    }
    let ini_path = &args[0];
    let src_arg = &args[1];
    let dest_dir = PathBuf::from(&args[2]);

    let ini_text = fs::read_to_string(ini_path)
        .map_err(|e| format!("kann INI-Datei '{ini_path}' nicht lesen: {e}"))?;
    let transforms = load_transforms(&ini_text, debug)?;
    eprintln!("{} Transform-Regeln geladen.", transforms.len());

    let src_files = collect_source_files(src_arg)?;
    if src_files.is_empty() {
        eprintln!("Keine .mdl-Quelldateien gefunden unter '{src_arg}'.");
        return Ok(());
    }
    fs::create_dir_all(&dest_dir)?;

    let mut written: HashSet<PathBuf> = HashSet::new();
    let mut processed = 0usize;
    let mut skipped_collisions = 0usize;

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
            if !wildcard_match(&t.match_pat, &stem) {
                continue;
            }
            let ext = src_path.extension().and_then(|e| e.to_str()).unwrap_or("mdl");
            let out_name = build_substitute(&stem, &t.substitute);
            let out_path = dest_dir.join(format!("{out_name}.{ext}"));

            if !written.insert(out_path.clone()) {
                eprintln!(
                    "Warnung: Zieldatei '{}' wurde in diesem Lauf bereits geschrieben, ueberspringe (aus {}).",
                    out_path.display(),
                    src_path.display()
                );
                skipped_collisions += 1;
                continue;
            }

            eprintln!("Verarbeite {} -> {}", src_path.display(), out_path.display());
            if let Err(e) = process_model(src_path, &stem, &out_name, &out_path, t) {
                eprintln!("  Fehler bei {}: {e}", src_path.display());
                continue;
            }
            processed += 1;
        }
        if !matched_any {
            eprintln!("Warnung: '{}' passt zu keiner Transform-Regel, uebersprungen. Nutze --debug fuer Details.", src_path.display());
                if debug {
                    eprintln!("  Modellname (Stamm): '{stem}'");
                    eprintln!("  Geladene match-Muster: {}", transforms.iter().map(|t| t.match_pat.as_str()).collect::<Vec<_>>().join(", "));
                }
        }

    eprintln!(
        "Fertig: {processed} Datei(en) geschrieben, {skipped_collisions} wegen Namenskollision uebersprungen."
    );
    Ok(())
}

fn collect_source_files(src_arg: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let path = Path::new(src_arg);
    if path.is_dir() {
        let mut out = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("mdl")).unwrap_or(false) {
                out.push(p);
            }
        }
        out.sort();
        Ok(out)
    } else {
        Ok(vec![path.to_path_buf()])
    }
}

/// Verarbeitet eine Quelldatei gegen eine einzelne Transform-Regel und
/// schreibt das Ergebnis atomar (ueber eine .tmp-Datei) nach `out_path`.
fn process_model(
    src_path: &Path,
    src_stem: &str,
    dest_stem: &str,
    out_path: &Path,
    t: &Transform,
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp_path = out_path.with_extension("mdl.tmp");
    let result = process_model_inner(src_path, src_stem, dest_stem, &tmp_path, t);
    match result {
        Ok(()) => {
            fs::rename(&tmp_path, out_path)?;
            Ok(())
        }
        Err(e) => {
            // Bug-Fix ggue. Original: keine halbfertige Datei liegen lassen.
            let _ = fs::remove_file(&tmp_path);
            Err(e)
        }
    }
}

fn process_model_inner(
    src_path: &Path,
    src_stem: &str,
    dest_stem: &str,
    tmp_path: &Path,
    t: &Transform,
) -> Result<(), Box<dyn std::error::Error>> {
    let reader = BufReader::new(fs::File::open(src_path)?);
    let mut out = fs::File::create(tmp_path)?;

    let mut last_bitmap = String::new();
    let mut lines = reader.lines();

    while let Some(line) = lines.next() {
        let line = line?;
        let trimmed = line.trim_start();
        let mut it = trimmed.split_whitespace();
        let keyword = it.next().unwrap_or("").to_lowercase();

        match keyword.as_str() {
            "verts" => {
                let n: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                writeln!(out, "{}", replace_no_case(&line, src_stem, dest_stem))?;
                write_transformed_verts(&mut lines, &mut out, n, t)?;
            }
            "tverts" => {
                let n: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                writeln!(out, "{}", replace_no_case(&line, src_stem, dest_stem))?;
                write_transformed_tverts(&mut lines, &mut out, n, t, &last_bitmap)?;
            }
            "bitmap" => {
                if let Some(name) = it.next() {
                    last_bitmap = name.to_lowercase();
                }
                writeln!(out, "{}", replace_no_case(&line, src_stem, dest_stem))?;
            }
            "position" => {
                let vals: Vec<f32> = it.filter_map(|s| s.parse().ok()).collect();
                let parsed = [
                    vals.first().copied().unwrap_or(0.0),
                    vals.get(1).copied().unwrap_or(0.0),
                    vals.get(2).copied().unwrap_or(0.0),
                ];
                let new_pos = match t.position {
                    PositionMode::Absolute(p) => p,
                    PositionMode::LikeVertex => t.apply_vertex(parsed).unwrap_or(parsed),
                };
                writeln!(out, "  position {:.7} {:.7} {:.7}", new_pos[0], new_pos[1], new_pos[2])?;
            }
            "filedependancy" | "filedependency" => {
                writeln!(out, "{line}")?;
            }
            _ => {
                writeln!(out, "{}", replace_no_case(&line, src_stem, dest_stem))?;
            }
        }
    }
    out.flush()?;
    Ok(())
}

fn write_transformed_verts(
    lines: &mut std::io::Lines<BufReader<fs::File>>,
    out: &mut fs::File,
    n: usize,
    t: &Transform,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..n {
        let Some(line) = lines.next() else {
            return Err("unerwartetes Dateiende in verts-Block".into());
        };
        let line = line?;
        let vals: Vec<f32> = line.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        if vals.len() < 3 {
            writeln!(out, "{line}")?;
            continue;
        }
        let v = [vals[0], vals[1], vals[2]];
        match t.apply_vertex(v) {
            Some(p) => writeln!(out, "    {:.7} {:.7} {:.7}", p[0], p[1], p[2])?,
            None => writeln!(out, "{line}")?,
        }
    }
    Ok(())
}

fn write_transformed_tverts(
    lines: &mut std::io::Lines<BufReader<fs::File>>,
    out: &mut fs::File,
    n: usize,
    t: &Transform,
    last_bitmap: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..n {
        let Some(line) = lines.next() else {
            return Err("unerwartetes Dateiende in tverts-Block".into());
        };
        let line = line?;
        let vals: Vec<f32> = line.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        if vals.len() < 2 {
            writeln!(out, "{line}")?;
            continue;
        }
        match t.apply_tvert(vals[0], vals[1], last_bitmap) {
            Some((x, y)) => writeln!(out, "    {:.7} {:.7} {:.7}", x, y, 0.0)?,
            None => writeln!(out, "{line}")?,
        }
    }
    Ok(())
}

/// Case-insensitiver Ersatz aller Vorkommen von `from` durch `to` in `line`
/// (Ersatz fuer ReplaceNoCase in IO.cpp).
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
        assert_eq!(replace_no_case("bitmap PM01_BELT001", "pm01_belt001", "pm01a_belt001"), "bitmap pm01a_belt001");
    }
}
