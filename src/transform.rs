// Port of the transform/INI logic from Transform.cpp + ini2.cpp.
//
// Deliberate simplifications compared to the original (ponytail: YAGNI):
// - No generic CMatrix class with reference counting; rotation applied directly
//   as three axis rotations on [f32; 3] (ref counting was
//   only necessary due to manual C++ memory management, redundant in Rust).
// - position=(x,y,z) is treated according to readme.txt as an ABSOLUTE displacement
//   (replaces the original position), not as an artifact of the
//   special CMatrix multiplication in the original.
// - wildcmp is the standard glob algorithm (functionally identical for
//   all patterns in the provided .ini files, without carrying over the edge cases of the
//   handwritten original implementation).

use std::collections::HashMap;
use std::fmt;

pub type Vec3 = [f32; 3];

#[derive(Debug)]
pub struct ParseError(pub String);
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ParseError {}

/// A loaded transform section ([s0], [s1], ...).
#[derive(Debug)]
pub struct Transform {
    pub match_pat: String,  // already lowercase
    pub substitute: String, // already lowercase

    pub scale: Vec3,
    pub rotate_deg: Vec3,
    pub translate: Vec3,
    pub min: Vec3,
    pub max: Vec3,

    pub tscale: [f32; 2],
    pub trotate_z_deg: f32,
    pub ttranslate: [f32; 2],
    pub tmin: [f32; 2],
    pub tmax: [f32; 2],
    pub tbitmap: Option<String>, // Some(name) = tvert transform only for this bitmap

    pub position: PositionMode,
}

#[derive(Debug, Clone, Copy)]
pub enum PositionMode {
    /// No position=(...) specified: position is treated like a normal vertex
    /// (Scale/Rotate/Translate, if within min/max).
    LikeVertex,
    /// position=(x,y,z) specified: absolute displacement (replaces the
    /// original position), see readme.txt.
    Absolute(Vec3),
}

impl Transform {
    fn is_just_copy(&self) -> bool {
        no_scale(self.scale) && no_rot(self.rotate_deg) && no_trans(self.translate)
    }
    fn is_just_tcopy(&self) -> bool {
        self.tscale[0] > 0.9999
            && self.tscale[0] < 1.0001
            && self.tscale[1] > 0.9999
            && self.tscale[1] < 1.0001
            && self.trotate_z_deg.abs() < 0.0001
            && self.ttranslate[0].abs() < 0.0001
            && self.ttranslate[1].abs() < 0.0001
    }
    fn in_range(&self, v: Vec3) -> bool {
        v[0] >= self.min[0]
            && v[0] <= self.max[0]
            && v[1] >= self.min[1]
            && v[1] <= self.max[1]
            && v[2] >= self.min[2]
            && v[2] <= self.max[2]
    }
    fn in_t_range(&self, x: f32, y: f32) -> bool {
        x >= self.tmin[0] && x <= self.tmax[0] && y >= self.tmin[1] && y <= self.tmax[1]
    }

    /// Applies Scale/Rotate/Translate to a vertex, unless it is a
    /// "just copy" and within min/max. Returns None if the line
    /// should be copied unchanged (original text).
    pub fn apply_vertex(&self, v: Vec3) -> Option<Vec3> {
        if self.is_just_copy() {
            return None;
        }
        if !self.in_range(v) {
            return None;
        }
        let mut p = v;
        if !no_scale(self.scale) {
            p = [
                p[0] * self.scale[0],
                p[1] * self.scale[1],
                p[2] * self.scale[2],
            ];
        }
        if !no_rot(self.rotate_deg) {
            p = rotate_xyz(p, self.rotate_deg);
        }
        if !no_trans(self.translate) {
            p = [
                p[0] + self.translate[0],
                p[1] + self.translate[1],
                p[2] + self.translate[2],
            ];
        }
        Some(p)
    }

    /// Applies the inverse-transpose of the linear part (Scale, Rotate) to a
    /// direction vector (normal or tangent xyz), then renormalizes. No
    /// translation (directions don't translate) and no min/max range gating:
    /// unlike verts, normals have no independent clipping in the original
    /// tool and no paired vertex position is buffered at this point in the
    /// line-by-line stream (see NWNArmory-Analysis.md, EE extensions).
    ///
    /// Correct handling of non-uniform scale requires the inverse transpose
    /// of the linear transform, not the transform itself, or the normal
    /// tilts away from the true surface normal on non-uniformly scaled
    /// geometry. apply_vertex's linear part is M = R * S (scale then
    /// rotate); since S is diagonal and R is orthogonal,
    /// invTranspose(M) = R * inv(S) -- divide by scale first, then apply the
    /// same rotation used for verts.
    pub fn apply_normal(&self, n: Vec3) -> Option<Vec3> {
        if no_scale(self.scale) && no_rot(self.rotate_deg) {
            return None;
        }
        let mut p = [
            safe_inv_scale(n[0], self.scale[0]),
            safe_inv_scale(n[1], self.scale[1]),
            safe_inv_scale(n[2], self.scale[2]),
        ];
        if !no_rot(self.rotate_deg) {
            p = rotate_xyz(p, self.rotate_deg);
        }
        Some(normalize_vec3(p))
    }

    /// Applies TScale/TRotate(Z only)/TTranslate to a texture coordinate.
    /// `last_bitmap` is the most recently seen `bitmap` statement (lowercase).
    pub fn apply_tvert(&self, x: f32, y: f32, last_bitmap: &str) -> Option<(f32, f32)> {
        if self.is_just_tcopy() {
            return None;
        }
        if let Some(tb) = &self.tbitmap {
            if tb != last_bitmap {
                return None;
            }
        }
        self.apply_tvert_raw(x, y)
    }

    /// Applies TScale/TRotate(Z only)/TTranslate to a texture coordinate on
    /// an EE extra UV channel (`tverts1`/`tverts2`/`tverts3`). Unlike the
    /// primary `tverts` channel, extra channels are NOT gated by
    /// `tbitmap`: that INI option predates multi-channel UVs, and every
    /// `tbitmap=` rule in the wild was written to restrict the primary
    /// texture stage, never a lightmap/normal-map UV set. Same
    /// scale/rotate/translate math otherwise.
    pub fn apply_tvert_extra(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        if self.is_just_tcopy() {
            return None;
        }
        self.apply_tvert_raw(x, y)
    }

    fn apply_tvert_raw(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        if !self.in_t_range(x, y) {
            return None;
        }
        let (mut px, mut py) = (x, y);
        if !(self.tscale[0] > 0.9999
            && self.tscale[0] < 1.0001
            && self.tscale[1] > 0.9999
            && self.tscale[1] < 1.0001)
        {
            px *= self.tscale[0];
            py *= self.tscale[1];
        }
        if self.trotate_z_deg.abs() >= 0.0001 {
            let r = self.trotate_z_deg.to_radians();
            let (s, c) = r.sin_cos();
            let (nx, ny) = (px * c + py * s, -px * s + py * c);
            px = nx;
            py = ny;
        }
        if self.ttranslate[0].abs() >= 0.0001 || self.ttranslate[1].abs() >= 0.0001 {
            px += self.ttranslate[0];
            py += self.ttranslate[1];
        }
        Some((px, py))
    }
}

fn no_scale(s: Vec3) -> bool {
    s.iter().all(|v| *v > 0.9999 && *v < 1.0001)
}
fn no_rot(r: Vec3) -> bool {
    r.iter().all(|v| v.abs() < 0.0001)
}
fn no_trans(t: Vec3) -> bool {
    t.iter().all(|v| v.abs() < 0.0001)
}

/// Divides a direction component by a scale factor for the inverse-transpose
/// normal transform. A near-zero scale axis (degenerate/flattening INI data)
/// would divide-by-zero into Inf/NaN; falling back to the un-divided
/// component keeps the result finite instead of producing garbage output.
fn safe_inv_scale(component: f32, scale: f32) -> f32 {
    if scale.abs() < 1e-6 {
        component
    } else {
        component / scale
    }
}

fn normalize_vec3(v: Vec3) -> Vec3 {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-8 {
        v
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

/// Applies rotation in degrees: X and Y axes negated (Max is
/// left-handed, see original comment in Transform.cpp), order
/// X then Y then Z, corresponding to the original multiplication order.
fn rotate_xyz(v: Vec3, deg: Vec3) -> Vec3 {
    let rx = (-deg[0]).to_radians();
    let ry = (-deg[1]).to_radians();
    let rz = deg[2].to_radians();
    let v = rotate_x(v, rx);
    let v = rotate_y(v, ry);
    rotate_z(v, rz)
}
fn rotate_x(v: Vec3, r: f32) -> Vec3 {
    let (s, c) = r.sin_cos();
    [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
}
fn rotate_y(v: Vec3, r: f32) -> Vec3 {
    let (s, c) = r.sin_cos();
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}
fn rotate_z(v: Vec3, r: f32) -> Vec3 {
    let (s, c) = r.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]]
}

// ---------------------------------------------------------------------
// INI parsing (replacement for ini2.cpp / GetPrivateProfileString)
// ---------------------------------------------------------------------

type Section = HashMap<String, String>;

fn parse_ini_with_path(
    path: &str,
    text: &str,
    _debug: bool,
) -> Result<HashMap<String, Section>, ParseError> {
    let mut sections: HashMap<String, Section> = HashMap::new();
    let mut current = String::from("global");

    for (index, raw_line) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw_line.trim();

        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') {
            let end = line.find(']').ok_or_else(|| {
                ParseError(format!(
                    "{path}:{line_no}: block header: unterminated header '{line}'"
                ))
            })?;

            if !line[end + 1..].trim().is_empty() {
                return Err(ParseError(format!(
                    "{path}:{line_no}: block header: unexpected content after '[...]'"
                )));
            }

            let name = line[1..end].trim();

            if name.is_empty() {
                return Err(ParseError(format!(
                    "{path}:{line_no}: block header: empty section name"
                )));
            }

            current = name.to_lowercase();

            if sections.contains_key(&current) {
                return Err(ParseError(format!(
                    "{path}:{line_no}: Section [{current}]: defined twice"
                )));
            }

            sections.insert(current.clone(), HashMap::new());
            continue;
        }

        let equals = line.find('=').ok_or_else(|| {
            ParseError(format!(
                "{path}:{line_no}: Section [{current}]: key/value without '='"
            ))
        })?;

        let key = line[..equals].trim().to_lowercase();

        if key.is_empty() {
            return Err(ParseError(format!(
                "{path}:{line_no}: Section [{current}]: empty key"
            )));
        }

        let value = line[equals + 1..].trim().to_string();
        let section = sections.entry(current.clone()).or_default();

        if section.contains_key(&key) {
            return Err(ParseError(format!(
                "{path}:{line_no}: Section [{current}], Key '{key}': defined twice"
            )));
        }

        section.insert(key, value);
    }

    Ok(sections)
}

fn get_str<'a>(section: &'a Section, key: &str, default: &'a str) -> &'a str {
    section
        .get(key)
        .map(|value| value.as_str())
        .unwrap_or(default)
}

fn parse_tuple(
    raw: &str,
    path: &str,
    section: &str,
    key: &str,
    expected: usize,
) -> Result<Vec<f32>, ParseError> {
    let text = raw.trim();

    let body = text
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| {
            ParseError(format!(
                "{path}: Section [{section}], Key '{key}': expected a tuple, got '{raw}'"
            ))
        })?;

    let values: Result<Vec<f32>, ParseError> = body
        .split(',')
        .enumerate()
        .map(|(index, part)| {
            let token = part.trim();

            if token.is_empty() {
                return Err(ParseError(format!(
                    "{path}: Section [{section}], Key '{key}': empty value at position {}",
                    index + 1
                )));
            }

            let value = token.parse::<f32>().map_err(|_| {
                ParseError(format!(
                    "{path}: Section [{section}], Key '{key}': invalid number '{token}'"
                ))
            })?;

            if !value.is_finite() {
                return Err(ParseError(format!(
                    "{path}: Section [{section}], Key '{key}': number '{token}' is not finite"
                )));
            }

            Ok(value)
        })
        .collect();

    let values = values?;

    if values.len() != expected {
        return Err(ParseError(format!(
            "{path}: Section [{section}], Key '{key}': expected {} value(s), got {}",
            expected,
            values.len()
        )));
    }

    Ok(values)
}

fn parse_vec3(
    section: &Section,
    key: &str,
    default: &str,
    path: &str,
    section_name: &str,
) -> Result<Vec3, ParseError> {
    let values = parse_tuple(get_str(section, key, default), path, section_name, key, 3)?;

    Ok([values[0], values[1], values[2]])
}

fn parse_vec2(
    section: &Section,
    key: &str,
    default: &str,
    path: &str,
    section_name: &str,
) -> Result<[f32; 2], ParseError> {
    let values = parse_tuple(get_str(section, key, default), path, section_name, key, 2)?;

    Ok([values[0], values[1]])
}

fn parse_scalar(
    section: &Section,
    key: &str,
    default: &str,
    path: &str,
    section_name: &str,
) -> Result<f32, ParseError> {
    Ok(parse_tuple(get_str(section, key, default), path, section_name, key, 1)?[0])
}

/// Loads all [s0]..[sN-1] sections based on [Global] nTransforms.
pub fn load_transforms_from_path(
    path: &str,
    ini_text: &str,
    debug: bool,
) -> Result<Vec<Transform>, ParseError> {
    const TRANSFORM_KEYS: &[&str] = &[
        "match",
        "substitute",
        "scale",
        "rotate",
        "translate",
        "minimum",
        "maximum",
        "tscale",
        "trotate",
        "ttranslate",
        "tminimum",
        "tmaximum",
        "tbitmap",
        "position",
    ];

    let sections = parse_ini_with_path(path, ini_text, debug)?;

    let global = sections
        .get("global")
        .ok_or_else(|| ParseError(format!("{path}: Section [Global]: missing")))?;

    let count_raw = global.get("ntransforms").ok_or_else(|| {
        ParseError(format!(
            "{path}: Section [Global], Key 'nTransforms': missing"
        ))
    })?;

    let count: usize = count_raw.parse().map_err(|_| {
        ParseError(format!(
            "{path}: Section [Global], Key 'nTransforms': invalid number '{count_raw}'"
        ))
    })?;

    if count == 0 {
        return Err(ParseError(format!(
            "{path}: Section [Global], Key 'nTransforms': must be greater than 0"
        )));
    }

    let mut transforms = Vec::with_capacity(count);

    for index in 0..count {
        let section_name = format!("s{index}");

        let section = sections
            .get(&section_name)
            .ok_or_else(|| ParseError(format!("{path}: Section [{section_name}]: missing")))?;

        for unknown_key in section
            .keys()
            .filter(|key| !TRANSFORM_KEYS.contains(&key.as_str()))
        {
            if debug {
                eprintln!(
                    "Warning: {path}: Section [{section_name}], Key '{unknown_key}': unknown"
                );
            }
        }

        let match_pat = section
            .get("match")
            .ok_or_else(|| {
                ParseError(format!(
                    "{path}: Section [{section_name}], Key 'match': missing"
                ))
            })?
            .to_lowercase();

        if match_pat.is_empty() {
            return Err(ParseError(format!(
                "{path}: Section [{section_name}], Key 'match': must not be empty"
            )));
        }

        let substitute = get_str(section, "substitute", "*").to_lowercase();

        let position_raw = parse_vec3(
            section,
            "position",
            "( -999.0 , -999.0 , -999.0 )",
            path,
            &section_name,
        )?;

        let position = if position_raw.iter().any(|component| *component > -998.0) {
            PositionMode::Absolute(position_raw)
        } else {
            PositionMode::LikeVertex
        };

        let tbitmap = section
            .get("tbitmap")
            .filter(|value| !value.is_empty())
            .map(|value| value.to_lowercase());

        transforms.push(Transform {
            match_pat,
            substitute,
            scale: parse_vec3(section, "scale", "( 1.0 , 1.0 , 1.0 )", path, &section_name)?,
            rotate_deg: parse_vec3(
                section,
                "rotate",
                "( 0.0 , 0.0 , 0.0 )",
                path,
                &section_name,
            )?,
            translate: parse_vec3(
                section,
                "translate",
                "( 0.0 , 0.0 , 0.0 )",
                path,
                &section_name,
            )?,
            min: parse_vec3(
                section,
                "minimum",
                "( -999.0 , -999.0 , -999.0 )",
                path,
                &section_name,
            )?,
            max: parse_vec3(
                section,
                "maximum",
                "( 999.0 , 999.0 , 999.0 )",
                path,
                &section_name,
            )?,
            tscale: parse_vec2(section, "tscale", "( 1.0 , 1.0 )", path, &section_name)?,
            trotate_z_deg: parse_scalar(section, "trotate", "( 0.0 )", path, &section_name)?,
            ttranslate: parse_vec2(section, "ttranslate", "( 0.0 , 0.0 )", path, &section_name)?,
            tmin: parse_vec2(
                section,
                "tminimum",
                "( -999.0 , -999.0 )",
                path,
                &section_name,
            )?,
            tmax: parse_vec2(
                section,
                "tmaximum",
                "( 999.0 , 999.0 )",
                path,
                &section_name,
            )?,
            tbitmap,
            position,
        });
    }

    Ok(transforms)
}

// ---------------------------------------------------------------------
// Wildcard matching (replacement for wildcmp in IO.cpp)
// ---------------------------------------------------------------------

/// Standard glob matching with `*` and `?`. Expects
/// lowercase strings (original also compares case-insensitively).
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let wild: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = text.chars().collect();
    let (mut wi, mut si) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut match_idx = 0usize;

    while si < s.len() {
        if wi < wild.len() && (wild[wi] == '?' || wild[wi] == s[si]) {
            wi += 1;
            si += 1;
        } else if wi < wild.len() && wild[wi] == '*' {
            star = Some(wi);
            match_idx = si;
            wi += 1;
        } else if let Some(star_idx) = star {
            wi = star_idx + 1;
            match_idx += 1;
            si = match_idx;
        } else {
            return false;
        }
    }
    while wi < wild.len() && wild[wi] == '*' {
        wi += 1;
    }
    wi == wild.len()
}

/// Builds the target model name from `old_name` (lowercase) and `substitute` pattern.
/// Corresponds to CIO::doSubstitute in IO.cpp.
pub fn build_substitute(old_name: &str, subst: &str) -> String {
    let mut out: Vec<char> = old_name.chars().collect();
    let mut idx = 0usize;
    for c in subst.chars() {
        match c {
            '?' => idx += 1,
            '*' => idx = out.len(),
            '\\' | '/' | ':' | '"' | '<' | '>' | '|' => { /* invalid wildcard character, skip */ }
            other => {
                if idx < out.len() {
                    out[idx] = other;
                } else {
                    out.push(other);
                }
                idx += 1;
            }
        }
    }
    if idx < out.len() {
        out.truncate(idx);
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_basic() {
        assert!(wildcard_match("pm??_belt???", "pm01_belt001"));
        assert!(!wildcard_match("pm??_belt???", "pf01_belt001"));
        assert!(wildcard_match("*", "anything"));
    }

    #[test]
    fn substitute_halfling() {
        // "pm" remains (2x '?'), 'a' replaces the race position (original digit '0'),
        // '*' carries over the rest incl. preserved phenotype digit '1'.
        // Yields "pma1_belt001" (Player, male, Halfling(a), phenotype 1) --
        // consistent with the real NWN naming convention p<gender><race><phenotype>.
        assert_eq!(build_substitute("pm01_belt001", "??a*"), "pma1_belt001");
    }

    fn identity_transform() -> Transform {
        Transform {
            match_pat: String::new(),
            substitute: String::new(),
            scale: [1.0, 1.0, 1.0],
            rotate_deg: [0.0, 0.0, 0.0],
            translate: [0.0, 0.0, 0.0],
            min: [-999.0, -999.0, -999.0],
            max: [999.0, 999.0, 999.0],
            tscale: [1.0, 1.0],
            trotate_z_deg: 0.0,
            ttranslate: [0.0, 0.0],
            tmin: [-999.0, -999.0],
            tmax: [999.0, 999.0],
            tbitmap: None,
            position: PositionMode::LikeVertex,
        }
    }

    #[test]
    fn normal_nonuniform_scale_uses_inverse_transpose() {
        // scale=(2,1,1): a naive "same transform as vertex" would scale the
        // normal by (2,1,1) too, tilting it the WRONG way. The correct
        // inverse-transpose divides by scale first: (1/2, 1, 1).
        let mut t = identity_transform();
        t.scale = [2.0, 1.0, 1.0];
        let n = t.apply_normal([1.0, 1.0, 0.0]).unwrap();

        let expected_len = (0.5f32 * 0.5 + 1.0f32 * 1.0).sqrt();
        let expected = [0.5 / expected_len, 1.0 / expected_len, 0.0];
        assert!((n[0] - expected[0]).abs() < 1e-5, "got {:?}", n);
        assert!((n[1] - expected[1]).abs() < 1e-5, "got {:?}", n);
        assert!(n[2].abs() < 1e-6, "got {:?}", n);

        // Sanity: the naive (wrong) approach would have produced (2,1,0)
        // normalized, i.e. X/Y swapped in dominance vs. the correct result.
        let wrong_len = (2.0f32 * 2.0 + 1.0f32 * 1.0).sqrt();
        let wrong = [2.0 / wrong_len, 1.0 / wrong_len, 0.0];
        assert!((n[0] - wrong[0]).abs() > 1e-3, "correction had no effect");
    }

    #[test]
    fn normal_just_copy_ignores_translation() {
        // Directions never translate; a pure-translation transform must be
        // a no-op for normals even though it is NOT a no-op for vertices.
        let mut t = identity_transform();
        t.translate = [5.0, -2.0, 0.0];
        assert!(t.apply_normal([0.0, 0.0, 1.0]).is_none());
    }

    #[test]
    fn normal_uniform_scale_keeps_direction() {
        // Uniform scale must not change the normal's direction, only its
        // magnitude, and apply_normal renormalizes anyway.
        let mut t = identity_transform();
        t.scale = [3.0, 3.0, 3.0];
        let n = t.apply_normal([0.0, 0.6, 0.8]).unwrap();
        assert!((n[1] - 0.6).abs() < 1e-5, "got {:?}", n);
        assert!((n[2] - 0.8).abs() < 1e-5, "got {:?}", n);
    }

    #[test]
    fn normal_zero_scale_axis_stays_finite() {
        // Degenerate (flattening) scale must not produce Inf/NaN.
        let mut t = identity_transform();
        t.scale = [0.0, 1.0, 1.0];
        let n = t.apply_normal([1.0, 0.0, 0.0]).unwrap();
        assert!(n.iter().all(|v| v.is_finite()), "got {:?}", n);
    }

    #[test]
    fn tvert_extra_ignores_tbitmap_filter() {
        // apply_tvert (primary channel) is gated by tbitmap; apply_tvert_extra
        // (tverts1/2/3) must NOT be, even with the exact same transform.
        let mut t = identity_transform();
        t.tscale = [2.0, 2.0];
        t.tbitmap = Some("some_other_bitmap".to_string());

        // Primary channel: bitmap doesn't match tbitmap -> untransformed (None).
        assert!(t.apply_tvert(0.5, 0.5, "current_bitmap").is_none());

        // Extra channel: same transform applies regardless of tbitmap.
        let (x, y) = t.apply_tvert_extra(0.5, 0.5).unwrap();
        assert!(
            (x - 1.0).abs() < 1e-5 && (y - 1.0).abs() < 1e-5,
            "got ({x}, {y})"
        );
    }

    #[test]
    fn malformed_ini_tuple_is_rejected_with_section_and_key() {
        let ini = "[Global]\nnTransforms=1\n[s0]\nmatch=x\nscale=(1, nope, 1)\n";

        let error = load_transforms_from_path("broken.ini", ini, false)
            .expect_err("malformed tuple must be rejected")
            .to_string();

        assert!(
            error.contains("broken.ini")
                && error.contains("Section [s0]")
                && error.contains("Key 'scale'")
                && error.contains("invalid number"),
            "{error}"
        );
    }

    #[test]
    fn ini_roundtrip() {
        let ini = r#"
[Global]
nTransforms=1

[s0]
match=pm??_belt???
substitute=??a*
Scale=(0.72, 0.72, 0.72)
"#;
        let transforms = load_transforms_from_path("<test>", ini, false).unwrap();
        assert_eq!(transforms.len(), 1);
        assert_eq!(transforms[0].scale, [0.72, 0.72, 0.72]);
    }
}
