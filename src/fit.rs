// fit.rs — computes best-fit scale/rotate/translate parameters between a
// source model and a target model (e.g. pmh0_chest001.mdl -> pfa0_chest001.mdl),
// so the user can drop the printed values straight into a transforms.ini
// section and decide themselves what to keep.
//
// Method: closed-form least squares (normal equations) + analytic 3x3/2x2
// decomposition. No linalg crate needed for problems this small.
//
// ponytail: assumes vertex correspondence by array index (source verts[i] <->
// target verts[i]), not nearest-neighbor matching. Holds for genuine
// NWNArmory-style race variants, where the transform never reorders/adds/
// removes vertices. Upgrade path if that assumption ever breaks: centroid
// alignment + nearest-neighbor correspondence (ICP) before fitting.
//
// The affine map recovered is v' = v*Scale*R + Translate (row-vector
// convention, matching apply_vertex in transform.rs). Decomposition exploits
// A*A^T = diag(scale^2), which holds exactly whenever the true map is
// diagonal-scale-then-rotate (true for every transforms.ini section).

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
}

/// Entry point for `nwnarmory --values <source.mdl> <target.mdl>`.
pub fn run_values(source_path: &str, target_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let src = read_geometry(Path::new(source_path))?;
    let tgt = read_geometry(Path::new(target_path))?;

    println!("; computed from {} -> {}", source_path, target_path);
    println!("; paste the lines you want into a transforms.ini [sN] section");
    println!();

    print_vertex_fit(&src, &tgt);
    println!();
    print_tvert_fit(&src, &tgt);
    println!();
    print_position(&src, &tgt);

    Ok(())
}

fn print_vertex_fit(src: &Geometry, tgt: &Geometry) {
    if src.verts.len() != tgt.verts.len() {
        eprintln!(
            "Warning: vertex count mismatch (source={}, target={}); models don't correspond 1:1, skipping scale/rotate/translate.",
            src.verts.len(),
            tgt.verts.len()
        );
        return;
    }
    if src.verts.len() < 3 {
        eprintln!(
            "Warning: only {} vertex(es); need at least 3 (non-collinear) to fit a 3-axis transform.",
            src.verts.len()
        );
        return;
    }
    match fit_affine3(&src.verts, &tgt.verts) {
        Some((a, t)) => {
            let (scale, rotate) = decompose_affine3(a);
            let (max_res, mean_res) = residual3(&src.verts, &tgt.verts, a, t);
            println!("scale=({:.4}, {:.4}, {:.4})", scale[0], scale[1], scale[2]);
            println!("rotate=({:.2}, {:.2}, {:.2})", rotate[0], rotate[1], rotate[2]);
            println!("translate=({:.4}, {:.4}, {:.4})", t[0], t[1], t[2]);
            println!(
                "; verts fit: n={} max_residual={:.5} mean_residual={:.5} (high residual = shear/non-uniform transform this tool can't model, or mismatched files)",
                src.verts.len(),
                max_res,
                mean_res
            );
        }
        None => eprintln!("Warning: vertex data is degenerate (e.g. all collinear), could not solve the 3-axis fit."),
    }
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
    let mut line_no = 0usize;

    while let Some(result) = lines.next() {
        line_no += 1;
        let line = result?;

        let mut it = line.split_whitespace();
        let keyword = it.next().unwrap_or("").to_lowercase();

        match keyword.as_str() {
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
                        geo.verts.push([values[0], values[1], values[2]]);
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
}
