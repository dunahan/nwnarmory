// fit.rs — computes best-fit transforms between source and target models.
// Skeleton-reference MDLs get a segment report instead of a misleading global
// transforms.ini fit; their rootdummy marker mesh is not body geometry.
//
// Method: closed-form least squares (normal equations) + analytic 3x3/2x2
// decomposition. No linalg crate needed for problems this small.
//
// Uses index correspondence when topology matches. Otherwise, a node-aware
// iterative closest point fallback estimates correspondences between meshes.
//
// The affine map recovered is v' = v*Scale*R + Translate (row-vector
// convention, matching apply_vertex in transform.rs). Decomposition exploits
// A*A^T = diag(scale^2), which holds exactly whenever the true map is
// diagonal-scale-then-rotate (true for every transforms.ini section).

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

type Mat3 = [[f64; 3]; 3];
type Mat2 = [[f64; 2]; 2];

#[derive(Default)]
struct Geometry {
    verts: Vec<[f64; 3]>,
    tverts: Vec<[f64; 2]>,
    positions: Vec<[f64; 3]>,
    meshes: Vec<MeshGeometry>,
}

#[derive(Default)]
struct MeshGeometry {
    node: String,
    verts: Vec<[f64; 3]>,
}

#[derive(Clone)]
struct NodeOffset {
    kind: String,
    name: String,
    parent: String,
    position: [f64; 3],
    has_position: bool,
}

#[derive(Default)]
struct Skeleton {
    model_name: String,
    supermodel: Option<String>,
    animation_scale: Option<f64>,
    nodes: Vec<NodeOffset>,
}

/// Returns the lowercase model stem used by `match` and `substitute`.
fn model_stem(path: &str) -> Result<String, Box<dyn std::error::Error>> {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map(str::to_lowercase)
        .ok_or_else(|| format!("cannot determine a model name from '{path}'").into())
}

/// Derives the compact wildcard pair used by the stock NWNArmory INI.
///
/// The part before the first '_' is the race/phenotype identifier. Once a
/// difference occurs there, the remainder of that identifier is variable
/// (`pm??`, `pf??`, ...). Numeric characters in the model-piece suffix are
/// also variable (`chest???`, `belt???`, ...). The substitute keeps the
/// source prefix with `?`, writes the changed target portion, and uses `*` to
/// carry the unchanged remainder through, e.g. `pm01_chest001` -> `pma1...`
/// yields `match=pm??_chest???` and `substitute=??a*`.
fn model_patterns(source: &str, target: &str) -> (String, String) {
    let src: Vec<char> = source.chars().collect();
    let tgt: Vec<char> = target.chars().collect();
    let separator = src.iter().position(|c| *c == '_');

    let (match_pattern, substitute) = match (separator, tgt.iter().position(|c| *c == '_')) {
        (Some(src_sep), Some(tgt_sep)) if src_sep == tgt_sep && src.len() == tgt.len() => {
            let differences: Vec<usize> = (0..src_sep).filter(|i| src[*i] != tgt[*i]).collect();
            let (wild_start, wild_end) = if let Some(first) = differences.first() {
                (*first, src_sep)
            } else {
                let digits: Vec<usize> =
                    (0..src_sep).filter(|i| src[*i].is_ascii_digit()).collect();
                match (digits.first(), digits.last()) {
                    (Some(first), Some(last)) => (*first, last + 1),
                    _ => (src_sep, src_sep),
                }
            };

            let mut head = String::new();
            head.extend(src[..wild_start].iter());
            head.extend(std::iter::repeat_n('?', wild_end - wild_start));

            let mut suffix = String::new();
            for c in &src[src_sep..] {
                suffix.push(if c.is_ascii_digit() { '?' } else { *c });
            }
            head.push_str(&suffix);

            let substitute =
                if let (Some(first), Some(last)) = (differences.first(), differences.last()) {
                    let mut value = "?".repeat(*first);
                    value.extend(tgt[*first..=*last].iter());
                    value.push('*');
                    value
                } else {
                    target.to_string()
                };
            (head, substitute)
        }
        _ => (source.to_string(), target.to_string()),
    };

    (match_pattern, substitute)
}

/// Entry point for `nwnarmory --values <source.mdl> <target.mdl>`.
pub fn run_values(source_path: &str, target_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let src = read_geometry(Path::new(source_path))?;
    let tgt = read_geometry(Path::new(target_path))?;
    let src_skeleton = read_skeleton(Path::new(source_path))?;
    let tgt_skeleton = read_skeleton(Path::new(target_path))?;

    // A body skeleton has useful per-bone measurements, but no geometry from
    // which to infer a single transforms.ini scale/rotate/translate.
    if is_skeleton_reference(&src) && is_skeleton_reference(&tgt) {
        print_skeleton_report(&src_skeleton, &tgt_skeleton);
        return Ok(());
    }

    let source_stem = model_stem(source_path)?;
    let target_stem = model_stem(target_path)?;

    println!("; Note: check the generated values and adjust appropriately");
    println!("; computed from {} -> {}", source_path, target_path);
    println!("; paste the lines you need into a transforms.ini [sN] section");
    let (match_pattern, substitute) = model_patterns(&source_stem, &target_stem);
    println!("; wildcard model-name mapping inferred from the two file names");
    println!("nTransforms=#");
    println!();
    println!("[s#]");
    println!("match={match_pattern}");
    println!("substitute={substitute}");
    println!();

    print_transform_fit(&src, &tgt, &src_skeleton, &tgt_skeleton)?;
    println!();
    print_tvert_fit(&src, &tgt);
    println!();
    print_position(&src, &tgt);

    Ok(())
}

/// Baseline rig MDLs can contain a tiny `rootdummy` marker mesh. It is not
/// body-part geometry and cannot support a useful global mesh fit.
fn is_skeleton_reference(geometry: &Geometry) -> bool {
    geometry.meshes.iter().all(|mesh| mesh.node == "rootdummy")
}

fn print_transform_fit(
    src: &Geometry,
    tgt: &Geometry,
    src_skeleton: &Skeleton,
    tgt_skeleton: &Skeleton,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut source_offsets = Vec::new();
    let mut target_offsets = Vec::new();
    let (matched, _, _, _) = matched_nodes(src_skeleton, tgt_skeleton);
    for (source, target) in matched {
        // A local offset is comparable only when both models explicitly give
        // it. An omitted position is not evidence for a zero-length bone.
        if source.has_position && target.has_position {
            source_offsets.push(source.position);
            target_offsets.push(target.position);
        }
    }

    let skeleton_fit = if source_offsets.len() >= 3 {
        fit_linear3(&source_offsets, &target_offsets).map(|a| {
            let (scale, rotate) = decompose_affine3(a);
            let (max_res, mean_res) = residual3(&source_offsets, &target_offsets, a, [0.0; 3]);
            (scale, rotate, max_res, mean_res, source_offsets.len())
        })
    } else {
        None
    };

    let vertex_fit = if src.verts.len() == tgt.verts.len() && src.verts.len() >= 3 {
        fit_affine3(&src.verts, &tgt.verts).map(|(a, translation)| {
            let (scale, rotate) = decompose_affine3(a);
            let (max_res, mean_res) = residual3(&src.verts, &tgt.verts, a, translation);
            (
                scale,
                rotate,
                translation,
                max_res,
                mean_res,
                src.verts.len(),
            )
        })
    } else {
        None
    };

    if let Some((scale, rotate, max_res, mean_res, count)) = skeleton_fit {
        println!("; scale/rotate fitted from {count} matching local bone offsets (same node name and parent)");
        if let Some((_, _, translation, v_max, v_mean, v_count)) = vertex_fit {
            println!("scale=({:.4}, {:.4}, {:.4})", scale[0], scale[1], scale[2]);
            println!(
                "rotate=({:.2}, {:.2}, {:.2})",
                rotate[0], rotate[1], rotate[2]
            );
            println!(
                "translate=({:.4}, {:.4}, {:.4})",
                translation[0], translation[1], translation[2]
            );
            println!("; bone-offset fit: max_residual={max_res:.5} mean_residual={mean_res:.5}");
            println!("; vertex fit diagnostic: n={v_count} max_residual={v_max:.5} mean_residual={v_mean:.5}");
        } else if let Some((a, translation, used)) = fit_meshes_icp(src, tgt, scale, rotate) {
            let (mesh_scale, mesh_rotate) = decompose_affine3(a);
            let (max_r, mean_r) = correspondence_residual(src, tgt, a, translation);
            println!(
                "scale=({:.4}, {:.4}, {:.4})",
                mesh_scale[0], mesh_scale[1], mesh_scale[2]
            );
            println!(
                "rotate=({:.2}, {:.2}, {:.2})",
                mesh_rotate[0], mesh_rotate[1], mesh_rotate[2]
            );
            println!(
                "translate=({:.4}, {:.4}, {:.4})",
                translation[0], translation[1], translation[2]
            );
            println!("; mesh ICP fit: n={used} max_residual={max_r:.5} mean_residual={mean_r:.5}");
            println!("; WARNING: topology differs; nearest-neighbor correspondences are approximate. Review these values before use.");
            eprintln!("Warning: topology differs; scale/rotate/translate use an approximate nearest-neighbor mesh fit. Review the generated values before applying them.");
            println!("; bone-offset fit diagnostic: max_residual={max_res:.5} mean_residual={mean_res:.5}");
        } else {
            println!("scale=({:.4}, {:.4}, {:.4})", scale[0], scale[1], scale[2]);
            println!(
                "rotate=({:.2}, {:.2}, {:.2})",
                rotate[0], rotate[1], rotate[2]
            );
            println!("translate=(0.0000, 0.0000, 0.0000)");
            println!("; bone-offset fit: max_residual={max_res:.5} mean_residual={mean_res:.5}");
            eprintln!("Warning: vertex blocks could not provide a transform fit; using bone offsets only.");
        }
    } else if let Some((scale, rotate, translation, max_res, mean_res, count)) = vertex_fit {
        println!("scale=({:.4}, {:.4}, {:.4})", scale[0], scale[1], scale[2]);
        println!(
            "rotate=({:.2}, {:.2}, {:.2})",
            rotate[0], rotate[1], rotate[2]
        );
        println!(
            "translate=({:.4}, {:.4}, {:.4})",
            translation[0], translation[1], translation[2]
        );
        println!("; verts fit: n={count} max_residual={max_res:.5} mean_residual={mean_res:.5}");
        println!(
            "; fewer than three compatible bone offsets; scale/rotate/translate use the vertex fit"
        );
    } else if let Some((a, translation, used)) = fit_meshes_icp(src, tgt, [1.0; 3], [0.0; 3]) {
        let (scale, rotate) = decompose_affine3(a);
        let (max_r, mean_r) = correspondence_residual(src, tgt, a, translation);
        println!("scale=({:.4}, {:.4}, {:.4})", scale[0], scale[1], scale[2]);
        println!(
            "rotate=({:.2}, {:.2}, {:.2})",
            rotate[0], rotate[1], rotate[2]
        );
        println!(
            "translate=({:.4}, {:.4}, {:.4})",
            translation[0], translation[1], translation[2]
        );
        println!("; mesh ICP fit: n={used} max_residual={max_r:.5} mean_residual={mean_r:.5}");
        println!("; WARNING: topology differs; nearest-neighbor correspondences are approximate. Review these values before use.");
        eprintln!("Warning: topology differs; scale/rotate/translate use an approximate nearest-neighbor mesh fit. Review the generated values before applying them.");
    } else {
        eprintln!("Warning: neither compatible bone offsets nor a usable mesh correspondence could provide a transform fit.");
    }
    Ok(())
}

fn read_skeleton(path: &Path) -> Result<Skeleton, Box<dyn std::error::Error>> {
    let file = fs::File::open(path)
        .map_err(|error| format!("cannot read '{}': {error}", path.display()))?;
    let mut skeleton = Skeleton::default();
    let mut current: Option<NodeOffset> = None;
    for (index, result) in BufReader::new(file).lines().enumerate() {
        let line_no = index + 1;
        let line = result?;
        let mut parts = line.split_whitespace();
        match parts.next().unwrap_or("").to_ascii_lowercase().as_str() {
            "newmodel" => {
                if let Some(name) = parts.next() {
                    skeleton.model_name = name.to_ascii_lowercase();
                }
            }
            "setsupermodel" => {
                let _model = parts.next();
                skeleton.supermodel = parts.next().map(str::to_ascii_lowercase);
            }
            "setanimationscale" => {
                if let Some(raw) = parts.next() {
                    let scale = raw.parse::<f64>().map_err(|_| {
                        format!(
                            "{}:{line_no}: invalid setanimationscale '{raw}'",
                            path.display()
                        )
                    })?;
                    if !scale.is_finite() {
                        return Err(format!(
                            "{}:{line_no}: setanimationscale must be finite",
                            path.display()
                        )
                        .into());
                    }
                    skeleton.animation_scale = Some(scale);
                }
            }
            "node" => {
                if let Some(node) = current.take() {
                    skeleton.nodes.push(node);
                }
                if let (Some(kind), Some(name)) = (parts.next(), parts.next()) {
                    current = Some(NodeOffset {
                        kind: kind.to_ascii_lowercase(),
                        name: name.to_ascii_lowercase(),
                        parent: String::new(),
                        position: [0.0; 3],
                        has_position: false,
                    });
                }
            }
            "parent" => {
                if let (Some(node), Some(parent)) = (current.as_mut(), parts.next()) {
                    node.parent = parent.to_ascii_lowercase();
                }
            }
            "position" => {
                if let Some(node) = current.as_mut() {
                    let values: Result<Vec<f64>, _> = parts.map(str::parse).collect();
                    let values = values.map_err(|_| {
                        format!("{}:{line_no}: invalid node position", path.display())
                    })?;
                    if values.len() != 3 || values.iter().any(|value| !value.is_finite()) {
                        return Err(format!(
                            "{}:{line_no}: exactly three finite node position values expected",
                            path.display()
                        )
                        .into());
                    }
                    node.position = [values[0], values[1], values[2]];
                    node.has_position = true;
                }
            }
            "endnode" => {
                if let Some(node) = current.take() {
                    skeleton.nodes.push(node);
                }
            }
            _ => {}
        }
    }
    if let Some(node) = current {
        skeleton.nodes.push(node);
    }
    for node in &mut skeleton.nodes {
        // The first child below each model root has a different parent name
        // in race variants, though it occupies the same model-root frame.
        if node.parent == skeleton.model_name {
            node.parent = "<model-root>".to_string();
        }
    }
    Ok(skeleton)
}

fn node_key(node: &NodeOffset) -> (String, String) {
    (node.name.clone(), node.parent.clone())
}

/// Collapse repeated skeleton node records. Several base models repeat the
/// same dummy tree for different sections; only the first copy carries its
/// position. If multiple copies provide conflicting positions, leave that
/// key unmatched rather than selecting an arbitrary transform.
fn unique_nodes(skeleton: &Skeleton) -> (HashMap<(String, String), &NodeOffset>, usize) {
    let mut groups: HashMap<(String, String), Vec<&NodeOffset>> = HashMap::new();
    for node in &skeleton.nodes {
        if node.parent.is_empty() {
            continue;
        }
        groups.entry(node_key(node)).or_default().push(node);
    }

    let mut unique = HashMap::new();
    let mut ambiguous = 0;
    for (key, group) in groups {
        let positioned: Vec<_> = group
            .iter()
            .copied()
            .filter(|node| node.has_position)
            .collect();
        let selected = match positioned.as_slice() {
            [one] => Some(*one),
            [] => group.first().copied(),
            many if many.iter().all(|node| node.position == many[0].position) => Some(many[0]),
            _ => None,
        };
        if let Some(node) = selected {
            unique.insert(key, node);
        } else {
            ambiguous += 1;
        }
    }
    (unique, ambiguous)
}

fn matched_nodes<'a>(
    source: &'a Skeleton,
    target: &'a Skeleton,
) -> (Vec<(&'a NodeOffset, &'a NodeOffset)>, usize, usize, usize) {
    let (source_nodes, source_ambiguous) = unique_nodes(source);
    let (target_nodes, target_ambiguous) = unique_nodes(target);
    let mut matched = Vec::new();
    let mut source_only = 0;
    for (key, source_node) in &source_nodes {
        if let Some(target_node) = target_nodes.get(key) {
            matched.push((*source_node, *target_node));
        } else {
            source_only += 1;
        }
    }
    matched.sort_by(|(a, _), (b, _)| (&a.parent, &a.name).cmp(&(&b.parent, &b.name)));
    let target_only = target_nodes
        .keys()
        .filter(|key| !source_nodes.contains_key(*key))
        .count();
    (
        matched,
        source_only,
        target_only,
        source_ambiguous + target_ambiguous,
    )
}

fn length(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn print_skeleton_report(source: &Skeleton, target: &Skeleton) {
    let (matched, source_only, target_only, ambiguous) = matched_nodes(source, target);
    println!(
        "; Skeleton comparison: {} -> {}",
        source.model_name, target.model_name
    );
    println!("; These measurements are rig hints, not a transforms.ini fit.");
    println!(
        "; Source: supermodel={}, animation scale={}",
        source.supermodel.as_deref().unwrap_or("unknown"),
        source
            .animation_scale
            .map_or_else(|| "unknown".to_string(), |v| format!("{v:.4}"))
    );
    println!(
        "; Target: supermodel={}, animation scale={}",
        target.supermodel.as_deref().unwrap_or("unknown"),
        target
            .animation_scale
            .map_or_else(|| "unknown".to_string(), |v| format!("{v:.4}"))
    );
    println!(
        "; Matched nodes: {}; source-only: {source_only}; target-only: {target_only}; ambiguous duplicate keys: {ambiguous}",
        matched.len()
    );
    println!("; Length ratio = target local offset length / source local offset length.");
    println!("; These ratios do not estimate robe width/depth; add or verify geometry scale from body-part proportions.");
    println!("; node | type | parent | source offset | target offset | source length | target length | length ratio");
    for (source_node, target_node) in matched {
        if !source_node.has_position && !target_node.has_position {
            continue;
        }
        let source_length = length(source_node.position);
        let target_length = length(target_node.position);
        let ratio = if source_node.has_position && target_node.has_position && source_length > 1e-9
        {
            format!("{:.4}", target_length / source_length)
        } else {
            "n/a".to_string()
        };
        println!(
            "; {} | {} | {} | ({:.6}, {:.6}, {:.6}) | ({:.6}, {:.6}, {:.6}) | {:.6} | {:.6} | {}",
            source_node.name,
            source_node.kind,
            source_node.parent,
            source_node.position[0],
            source_node.position[1],
            source_node.position[2],
            target_node.position[0],
            target_node.position[1],
            target_node.position[2],
            source_length,
            target_length,
            ratio
        );
    }
}

fn fit_linear3(src: &[[f64; 3]], tgt: &[[f64; 3]]) -> Option<Mat3> {
    if src.len() != tgt.len() || src.is_empty() {
        return None;
    }
    let mut m = [[0.0; 3]; 3];
    let mut nn = [[0.0; 3]; 3];
    for (source, target) in src.iter().zip(tgt) {
        for j in 0..3 {
            for k in 0..3 {
                m[j][k] += source[j] * source[k];
                nn[j][k] += source[j] * target[k];
            }
        }
    }
    Some(mat3_mul(mat3_inverse(m)?, nn))
}

fn print_tvert_fit(src: &Geometry, tgt: &Geometry) {
    if src.tverts.len() != tgt.tverts.len() {
        eprintln!(
            "Warning: tvert count mismatch (source={}, target={}), skipping texture fit.",
            src.tverts.len(),
            tgt.tverts.len()
        );
        return;
    }
    if src.tverts.len() < 2 {
        eprintln!(
            "Warning: only {} tvert(s); need at least 2 to fit a texture transform.",
            src.tverts.len()
        );
        return;
    }
    match fit_affine2(&src.tverts, &tgt.tverts) {
        Some((a, t)) => {
            let (tscale, trotate) = decompose_affine2(a);
            let (max_res, mean_res) = residual2(&src.tverts, &tgt.tverts, a, t);
            println!("tscale=({:.4}, {:.4})", tscale[0], tscale[1]);
            println!("trotate=({:.2})", trotate);
            println!("ttranslate=({:.4}, {:.4})", t[0], t[1]);
            println!(
                "; tverts fit: n={} max_residual={:.5} mean_residual={:.5}",
                src.tverts.len(),
                max_res,
                mean_res
            );
        }
        None => eprintln!("Warning: tvert data is degenerate, could not solve the texture fit."),
    }
}

fn print_position(src: &Geometry, tgt: &Geometry) {
    match (src.positions.first(), tgt.positions.first()) {
        (Some(sp), Some(tp)) => {
            // position=(x,y,z) is an ABSOLUTE value per readme.txt, not a
            // delta transform, so no fitting is needed: report the target
            // value directly.
            println!("position=({:.4}, {:.4}, {:.4})", tp[0], tp[1], tp[2]);
            println!(
                "; source position was ({:.4}, {:.4}, {:.4})",
                sp[0], sp[1], sp[2]
            );
            if src.positions.len() > 1 || tgt.positions.len() > 1 {
                eprintln!("Note: file(s) contain more than one 'position' line; only the first pair was compared.");
            }
        }
        _ => eprintln!("Warning: no 'position' line found in one or both files, skipping."),
    }
}

// ---------------------------------------------------------------------
// Geometry extraction
// ---------------------------------------------------------------------

fn read_geometry(path: &Path) -> Result<Geometry, Box<dyn std::error::Error>> {
    let file = fs::File::open(path)
        .map_err(|error| format!("cannot read '{}': {error}", path.display()))?;

    let mut lines = BufReader::new(file).lines();
    let mut geo = Geometry::default();
    let mut current_node = String::new();
    let mut line_no = 0usize;

    while let Some(result) = lines.next() {
        line_no += 1;
        let line = result?;

        let mut it = line.split_whitespace();
        let keyword = it.next().unwrap_or("").to_lowercase();

        match keyword.as_str() {
            "node" => current_node = it.nth(1).unwrap_or("").to_ascii_lowercase(),
            "endnode" => current_node.clear(),
            "verts" | "tverts" => {
                let block = keyword.as_str();

                let raw_count = it.next().ok_or_else(|| {
                    format!(
                        "{}:{}: Block '{}': Count missing",
                        path.display(),
                        line_no,
                        block
                    )
                })?;

                let count: usize = raw_count.parse().map_err(|_| {
                    format!(
                        "{}:{}: Block '{}': invalid count '{}'",
                        path.display(),
                        line_no,
                        block,
                        raw_count
                    )
                })?;

                let needed = if block == "verts" { 3 } else { 2 };

                for _ in 0..count {
                    line_no += 1;

                    let item = lines.next().ok_or_else(|| {
                        format!(
                            "{}:{}: Block '{}': unexpected end of file",
                            path.display(),
                            line_no,
                            block
                        )
                    })??;

                    let values: Result<Vec<f64>, _> =
                        item.split_whitespace().map(str::parse).collect();

                    let values = values.map_err(|_| {
                        format!(
                            "{}:{}: Block '{}': invalid number",
                            path.display(),
                            line_no,
                            block
                        )
                    })?;

                    if values.len() < needed || values.iter().any(|value| !value.is_finite()) {
                        return Err(format!(
                            "{}:{}: Block '{}': at least {} finite numbers expected",
                            path.display(),
                            line_no,
                            block,
                            needed
                        )
                        .into());
                    }

                    if block == "verts" {
                        let point = [values[0], values[1], values[2]];
                        geo.verts.push(point);
                        let index = geo
                            .meshes
                            .iter()
                            .position(|mesh| mesh.node == current_node)
                            .unwrap_or_else(|| {
                                geo.meshes.push(MeshGeometry {
                                    node: current_node.clone(),
                                    verts: Vec::new(),
                                });
                                geo.meshes.len() - 1
                            });
                        geo.meshes[index].verts.push(point);
                    } else {
                        geo.tverts.push([values[0], values[1]]);
                    }
                }
            }

            "position" => {
                let values: Result<Vec<f64>, _> = it.map(str::parse).collect();

                let values = values.map_err(|_| {
                    format!(
                        "{}:{}: Block 'position': invalid number",
                        path.display(),
                        line_no
                    )
                })?;

                if values.len() != 3 || values.iter().any(|value| !value.is_finite()) {
                    return Err(format!(
                        "{}:{}: Block 'position': exactly 3 finite numbers expected",
                        path.display(),
                        line_no
                    )
                    .into());
                }

                geo.positions.push([values[0], values[1], values[2]]);
            }

            _ => {}
        }
    }

    Ok(geo)
}

/// Fits a shared transform using node-matched meshes and ICP correspondences.
/// Matching node names are preferred; differently named meshes are paired only
/// when each model has exactly one unmatched non-empty mesh.
fn fit_meshes_icp(
    src: &Geometry,
    tgt: &Geometry,
    seed_scale: [f64; 3],
    seed_rotate: [f64; 3],
) -> Option<(Mat3, [f64; 3], usize)> {
    let pairs = paired_meshes(src, tgt);
    if pairs.is_empty() {
        return None;
    }
    let mut source_points = Vec::new();
    let mut target_points = Vec::new();
    for (mesh_id, (s, t)) in pairs.into_iter().enumerate() {
        if s.verts.len() < 4 || t.verts.len() < 4 {
            continue;
        }
        source_points.extend(s.verts.iter().map(|p| (mesh_id, *p)));
        target_points.extend(t.verts.iter().map(|p| (mesh_id, *p)));
    }
    if source_points.len() < 4 || target_points.len() < 4 {
        return None;
    }
    // Bound quadratic nearest-neighbor work for unusually dense meshes.
    let source_points = sample_tagged_points(&source_points, 1200);
    let target_points = sample_tagged_points(&target_points, 1200);
    let mut a = transform_matrix(seed_scale, seed_rotate);
    let mut translation = [0.0; 3];
    if seed_scale == [1.0; 3] && seed_rotate == [0.0; 3] {
        let source_centers: Vec<_> = source_points.iter().map(|(_, point)| *point).collect();
        let target_centers: Vec<_> = target_points.iter().map(|(_, point)| *point).collect();
        translation = sub3(centroid3(&target_centers), centroid3(&source_centers));
    }
    let mut correspondences = Vec::new();
    for _ in 0..30 {
        let mut distances: Vec<(f64, [f64; 3], [f64; 3])> = source_points
            .iter()
            .map(|(mesh_id, s)| {
                let p = add3(apply3_linear(*s, a), translation);
                let q = target_points
                    .iter()
                    .filter(|(target_mesh_id, _)| target_mesh_id == mesh_id)
                    .map(|(_, point)| point)
                    .min_by(|x, y| dist3(p, **x).total_cmp(&dist3(p, **y)))
                    .copied()
                    .unwrap();
                (dist3(p, q), *s, q)
            })
            .collect();
        distances.sort_by(|x, y| x.0.total_cmp(&y.0));
        let keep = ((distances.len() as f64 * 0.8).ceil() as usize).max(4);
        distances.truncate(keep);
        let fit_src: Vec<_> = distances.iter().map(|x| x.1).collect();
        let fit_tgt: Vec<_> = distances.iter().map(|x| x.2).collect();
        let (next_a, next_t) = fit_affine3(&fit_src, &fit_tgt)?;
        let delta = distances.iter().map(|x| x.0).sum::<f64>() / distances.len() as f64;
        correspondences = distances;
        a = next_a;
        translation = next_t;
        if delta < 1e-7 {
            break;
        }
    }
    if correspondences.len() < 4 {
        return None;
    }
    let (scale, rotate) = decompose_affine3(a);
    let represented_a = transform_matrix(scale, rotate);
    let mut represented_t = [0.0; 3];
    for (_, s, q) in &correspondences {
        let delta = sub3(*q, apply3_linear(*s, represented_a));
        represented_t = add3(represented_t, delta);
    }
    represented_t = [
        represented_t[0] / correspondences.len() as f64,
        represented_t[1] / correspondences.len() as f64,
        represented_t[2] / correspondences.len() as f64,
    ];
    Some((represented_a, represented_t, correspondences.len()))
}

fn paired_meshes<'a>(
    src: &'a Geometry,
    tgt: &'a Geometry,
) -> Vec<(&'a MeshGeometry, &'a MeshGeometry)> {
    let mut pairs: Vec<(&MeshGeometry, &MeshGeometry)> = Vec::new();
    let mut used_target = vec![false; tgt.meshes.len()];
    for sm in src.meshes.iter().filter(|m| !m.verts.is_empty()) {
        if let Some((i, tm)) = tgt
            .meshes
            .iter()
            .enumerate()
            .find(|(i, m)| !used_target[*i] && !m.verts.is_empty() && m.node == sm.node)
        {
            used_target[i] = true;
            pairs.push((sm, tm));
        }
    }
    let unmatched_src: Vec<_> = src
        .meshes
        .iter()
        .filter(|m| !m.verts.is_empty() && !pairs.iter().any(|(s, _)| std::ptr::eq(*s, *m)))
        .collect();
    let unmatched_tgt: Vec<_> = tgt
        .meshes
        .iter()
        .enumerate()
        .filter(|(i, m)| !used_target[*i] && !m.verts.is_empty())
        .map(|(_, m)| m)
        .collect();
    if unmatched_src.len() == 1 && unmatched_tgt.len() == 1 {
        pairs.push((unmatched_src[0], unmatched_tgt[0]));
    }
    pairs
}

fn sample_tagged_points(points: &[(usize, [f64; 3])], limit: usize) -> Vec<(usize, [f64; 3])> {
    if points.len() <= limit {
        return points.to_vec();
    }
    let step = points.len() as f64 / limit as f64;
    (0..limit)
        .map(|i| points[(i as f64 * step) as usize])
        .collect()
}

fn transform_matrix(scale: [f64; 3], rotate: [f64; 3]) -> Mat3 {
    let basis = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        let mut p = [
            basis[i][0] * scale[0],
            basis[i][1] * scale[1],
            basis[i][2] * scale[2],
        ];
        for (axis, degrees) in [(-rotate[0], 0), (-rotate[1], 1), (rotate[2], 2)] {
            let (s, c) = axis.to_radians().sin_cos();
            p = match degrees {
                0 => [p[0], p[1] * c - p[2] * s, p[1] * s + p[2] * c],
                1 => [p[0] * c + p[2] * s, p[1], -p[0] * s + p[2] * c],
                _ => [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]],
            };
        }
        out[i] = p;
    }
    out
}

fn correspondence_residual(src: &Geometry, tgt: &Geometry, a: Mat3, t: [f64; 3]) -> (f64, f64) {
    let pairs = mesh_correspondence_points(src, tgt, a, t);
    let distances: Vec<_> = pairs
        .into_iter()
        .map(|(s, q)| dist3(add3(apply3_linear(s, a), t), q))
        .collect();
    if distances.is_empty() {
        return (0.0, 0.0);
    }
    (
        distances.iter().copied().fold(0.0, f64::max),
        distances.iter().sum::<f64>() / distances.len() as f64,
    )
}

fn mesh_correspondence_points(
    src: &Geometry,
    tgt: &Geometry,
    a: Mat3,
    t: [f64; 3],
) -> Vec<([f64; 3], [f64; 3])> {
    let mut out = Vec::new();
    for (sm, tm) in paired_meshes(src, tgt) {
        out.extend(sm.verts.iter().map(|s| {
            let p = add3(apply3_linear(*s, a), t);
            (
                *s,
                *tm.verts
                    .iter()
                    .min_by(|x, y| dist3(p, **x).total_cmp(&dist3(p, **y)))
                    .unwrap(),
            )
        }));
    }
    out
}

// ---------------------------------------------------------------------
// 3D affine fit + decomposition
// ---------------------------------------------------------------------

fn fit_affine3(src: &[[f64; 3]], tgt: &[[f64; 3]]) -> Option<(Mat3, [f64; 3])> {
    let n = src.len();
    if n == 0 {
        return None;
    }
    let cs = centroid3(src);
    let ct = centroid3(tgt);

    let mut m = [[0.0; 3]; 3]; // sum outer(ps, ps)
    let mut nn = [[0.0; 3]; 3]; // sum outer(ps, pt)
    for i in 0..n {
        let ps = sub3(src[i], cs);
        let pt = sub3(tgt[i], ct);
        for j in 0..3 {
            for k in 0..3 {
                m[j][k] += ps[j] * ps[k];
                nn[j][k] += ps[j] * pt[k];
            }
        }
    }
    let m_inv = mat3_inverse(m)?;
    let a = mat3_mul(m_inv, nn);
    let t = sub3(ct, apply3_linear(cs, a));
    Some((a, t))
}

fn decompose_affine3(a: Mat3) -> ([f64; 3], [f64; 3]) {
    let at = mat3_transpose(a);
    let aat = mat3_mul(a, at);
    let scale = [
        aat[0][0].max(0.0).sqrt(),
        aat[1][1].max(0.0).sqrt(),
        aat[2][2].max(0.0).sqrt(),
    ];

    let eps = 1e-9;
    let s_inv = [
        if scale[0] > eps { 1.0 / scale[0] } else { 0.0 },
        if scale[1] > eps { 1.0 / scale[1] } else { 0.0 },
        if scale[2] > eps { 1.0 / scale[2] } else { 0.0 },
    ];
    let s_inv_diag: Mat3 = [
        [s_inv[0], 0.0, 0.0],
        [0.0, s_inv[1], 0.0],
        [0.0, 0.0, s_inv[2]],
    ];
    let r = mat3_mul(at, s_inv_diag); // R = A^T * S^-1

    let ry = (-r[2][0]).clamp(-1.0, 1.0).asin();
    let cy = ry.cos();
    // ponytail: gimbal lock (ry ~= +-90 deg) makes roll/yaw ambiguous; we
    // arbitrarily fold everything into rx and set rz=0 rather than crash.
    // Upgrade path if this ever bites: quaternion-based decomposition.
    let (rx, rz) = if cy.abs() > 1e-6 {
        (r[2][1].atan2(r[2][2]), r[1][0].atan2(r[0][0]))
    } else {
        ((-r[1][2]).atan2(r[1][1]), 0.0)
    };

    let rad2deg = 180.0 / std::f64::consts::PI;
    // Matches transform.rs::rotate_xyz: rx/ry are negated on the way in
    // (Max's left-handed coordinate system), rz is not.
    (scale, [-rx * rad2deg, -ry * rad2deg, rz * rad2deg])
}

fn residual3(src: &[[f64; 3]], tgt: &[[f64; 3]], a: Mat3, t: [f64; 3]) -> (f64, f64) {
    let mut max_r = 0.0f64;
    let mut sum_r = 0.0f64;
    for (s, q) in src.iter().zip(tgt.iter()) {
        let pred = add3(apply3_linear(*s, a), t);
        let d = dist3(pred, *q);
        max_r = max_r.max(d);
        sum_r += d;
    }
    (max_r, sum_r / src.len() as f64)
}

// ---------------------------------------------------------------------
// 2D affine fit + decomposition (texture coordinates)
// ---------------------------------------------------------------------

fn fit_affine2(src: &[[f64; 2]], tgt: &[[f64; 2]]) -> Option<(Mat2, [f64; 2])> {
    let n = src.len();
    if n == 0 {
        return None;
    }
    let cs = centroid2(src);
    let ct = centroid2(tgt);

    let mut m = [[0.0; 2]; 2];
    let mut nn = [[0.0; 2]; 2];
    for i in 0..n {
        let ps = sub2(src[i], cs);
        let pt = sub2(tgt[i], ct);
        for j in 0..2 {
            for k in 0..2 {
                m[j][k] += ps[j] * ps[k];
                nn[j][k] += ps[j] * pt[k];
            }
        }
    }
    let m_inv = mat2_inverse(m)?;
    let a = mat2_mul(m_inv, nn);
    let t = sub2(ct, apply2_linear(cs, a));
    Some((a, t))
}

fn decompose_affine2(a: Mat2) -> ([f64; 2], f64) {
    let at = mat2_transpose(a);
    let aat = mat2_mul(a, at);
    let scale = [aat[0][0].max(0.0).sqrt(), aat[1][1].max(0.0).sqrt()];

    let eps = 1e-9;
    let s_inv = [
        if scale[0] > eps { 1.0 / scale[0] } else { 0.0 },
        if scale[1] > eps { 1.0 / scale[1] } else { 0.0 },
    ];
    let s_inv_diag: Mat2 = [[s_inv[0], 0.0], [0.0, s_inv[1]]];
    let r = mat2_mul(at, s_inv_diag);
    let angle_deg = r[0][1].atan2(r[0][0]).to_degrees();
    (scale, angle_deg)
}

fn residual2(src: &[[f64; 2]], tgt: &[[f64; 2]], a: Mat2, t: [f64; 2]) -> (f64, f64) {
    let mut max_r = 0.0f64;
    let mut sum_r = 0.0f64;
    for (s, q) in src.iter().zip(tgt.iter()) {
        let pred = add2(apply2_linear(*s, a), t);
        let d = dist2(pred, *q);
        max_r = max_r.max(d);
        sum_r += d;
    }
    (max_r, sum_r / src.len() as f64)
}

// ---------------------------------------------------------------------
// Small vector/matrix helpers (no external crate needed for 2x2/3x3)
// ---------------------------------------------------------------------

fn centroid3(pts: &[[f64; 3]]) -> [f64; 3] {
    let n = pts.len() as f64;
    let mut c = [0.0; 3];
    for p in pts {
        c[0] += p[0];
        c[1] += p[1];
        c[2] += p[2];
    }
    [c[0] / n, c[1] / n, c[2] / n]
}
fn centroid2(pts: &[[f64; 2]]) -> [f64; 2] {
    let n = pts.len() as f64;
    let mut c = [0.0; 2];
    for p in pts {
        c[0] += p[0];
        c[1] += p[1];
    }
    [c[0] / n, c[1] / n]
}
fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub2(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn add2(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] + b[0], a[1] + b[1]]
}
fn dist3(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
fn dist2(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn apply3_linear(v: [f64; 3], a: Mat3) -> [f64; 3] {
    [
        v[0] * a[0][0] + v[1] * a[1][0] + v[2] * a[2][0],
        v[0] * a[0][1] + v[1] * a[1][1] + v[2] * a[2][1],
        v[0] * a[0][2] + v[1] * a[1][2] + v[2] * a[2][2],
    ]
}
fn apply2_linear(v: [f64; 2], a: Mat2) -> [f64; 2] {
    [
        v[0] * a[0][0] + v[1] * a[1][0],
        v[0] * a[0][1] + v[1] * a[1][1],
    ]
}

fn mat3_mul(a: Mat3, b: Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}
fn mat3_transpose(a: Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in a.iter().enumerate() {
        for (j, &value) in row.iter().enumerate() {
            out[j][i] = value;
        }
    }
    out
}
fn mat3_inverse(a: Mat3) -> Option<Mat3> {
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        [
            (a[1][1] * a[2][2] - a[1][2] * a[2][1]) * d,
            (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * d,
            (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * d,
        ],
        [
            (a[1][2] * a[2][0] - a[1][0] * a[2][2]) * d,
            (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * d,
            (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * d,
        ],
        [
            (a[1][0] * a[2][1] - a[1][1] * a[2][0]) * d,
            (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * d,
            (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * d,
        ],
    ])
}

fn mat2_mul(a: Mat2, b: Mat2) -> Mat2 {
    [
        [
            a[0][0] * b[0][0] + a[0][1] * b[1][0],
            a[0][0] * b[0][1] + a[0][1] * b[1][1],
        ],
        [
            a[1][0] * b[0][0] + a[1][1] * b[1][0],
            a[1][0] * b[0][1] + a[1][1] * b[1][1],
        ],
    ]
}
fn mat2_transpose(a: Mat2) -> Mat2 {
    [[a[0][0], a[1][0]], [a[0][1], a[1][1]]]
}
fn mat2_inverse(a: Mat2) -> Option<Mat2> {
    let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([[a[1][1] * d, -a[0][1] * d], [-a[1][0] * d, a[0][0] * d]])
}

// ---------------------------------------------------------------------
// Self-check: roundtrip synthetic data through the exact math used in
// transform.rs (duplicated here on purpose — a public re-export from
// transform.rs just for a test isn't worth the API surface).
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn t_rotate_x(v: [f64; 3], r: f64) -> [f64; 3] {
        let (s, c) = r.sin_cos();
        [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
    }
    fn t_rotate_y(v: [f64; 3], r: f64) -> [f64; 3] {
        let (s, c) = r.sin_cos();
        [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
    }
    fn t_rotate_z(v: [f64; 3], r: f64) -> [f64; 3] {
        let (s, c) = r.sin_cos();
        [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]]
    }
    fn t_apply(
        v: [f64; 3],
        scale: [f64; 3],
        rotate_deg: [f64; 3],
        translate: [f64; 3],
    ) -> [f64; 3] {
        let mut p = [v[0] * scale[0], v[1] * scale[1], v[2] * scale[2]];
        p = t_rotate_x(p, -rotate_deg[0].to_radians());
        p = t_rotate_y(p, -rotate_deg[1].to_radians());
        p = t_rotate_z(p, rotate_deg[2].to_radians());
        [
            p[0] + translate[0],
            p[1] + translate[1],
            p[2] + translate[2],
        ]
    }

    #[test]
    fn fit_affine3_roundtrip() {
        let src = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.5, 0.3, 0.7],
            [-0.4, 0.9, 0.2],
        ];
        let scale = [0.8, 1.2, 0.95];
        let rotate = [12.0, -7.0, 25.0];
        let translate = [1.5, -0.3, 0.2];
        let tgt: Vec<[f64; 3]> = src
            .iter()
            .map(|v| t_apply(*v, scale, rotate, translate))
            .collect();

        let (a, t) = fit_affine3(&src, &tgt).expect("fit should succeed");
        let (got_scale, got_rot) = decompose_affine3(a);

        for i in 0..3 {
            assert!((got_scale[i] - scale[i]).abs() < 1e-6, "scale[{i}]");
            assert!(
                (got_rot[i] - rotate[i]).abs() < 1e-4,
                "rot[{i}]: got {} want {}",
                got_rot[i],
                rotate[i]
            );
            assert!((t[i] - translate[i]).abs() < 1e-6, "t[{i}]");
        }
    }

    #[test]
    fn fit_affine2_roundtrip() {
        let src = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [0.3, 0.8]];
        let tscale = [0.6, 1.4];
        let trot_deg = 18.0f64;
        let ttrans = [0.2, -0.1];
        let r = trot_deg.to_radians();
        let (s, c) = r.sin_cos();
        let tgt: Vec<[f64; 2]> = src
            .iter()
            .map(|v| {
                let sv = [v[0] * tscale[0], v[1] * tscale[1]];
                let rv = [sv[0] * c + sv[1] * s, -sv[0] * s + sv[1] * c];
                [rv[0] + ttrans[0], rv[1] + ttrans[1]]
            })
            .collect();

        let (a2, t2) = fit_affine2(&src, &tgt).expect("fit should succeed");
        let (got_scale, got_rot) = decompose_affine2(a2);

        assert!((got_scale[0] - tscale[0]).abs() < 1e-6);
        assert!((got_scale[1] - tscale[1]).abs() < 1e-6);
        assert!((got_rot - trot_deg).abs() < 1e-4);
        assert!((t2[0] - ttrans[0]).abs() < 1e-6);
        assert!((t2[1] - ttrans[1]).abs() < 1e-6);
    }

    #[test]
    fn model_stem_is_lowercase_and_without_extension() {
        assert_eq!(
            model_stem("Models/PMH0_Chest001.MDL").unwrap(),
            "pmh0_chest001"
        );
        assert_eq!(model_stem("pfa0_chest001.mdl").unwrap(), "pfa0_chest001");
    }

    #[test]
    fn model_patterns_follow_stock_ini_wildcards() {
        let patterns = model_patterns("pm01_chest001", "pma1_chest001");
        assert_eq!(patterns.0, "pm??_chest???");
        assert_eq!(patterns.1, "??a*");
    }
}
