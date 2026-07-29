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
pub struct Transform {
    pub match_pat: String,   // already lowercase
    pub substitute: String,  // already lowercase

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

#[derive(Clone, Copy)]
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
        self.tscale[0] > 0.9999 && self.tscale[0] < 1.0001
            && self.tscale[1] > 0.9999 && self.tscale[1] < 1.0001
            && self.trotate_z_deg.abs() < 0.0001
            && self.ttranslate[0].abs() < 0.0001
            && self.ttranslate[1].abs() < 0.0001
    }
    fn in_range(&self, v: Vec3) -> bool {
        v[0] >= self.min[0] && v[0] <= self.max[0]
            && v[1] >= self.min[1] && v[1] <= self.max[1]
            && v[2] >= self.min[2] && v[2] <= self.max[2]
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
            p = [p[0] * self.scale[0], p[1] * self.scale[1], p[2] * self.scale[2]];
        }
        if !no_rot(self.rotate_deg) {
            p = rotate_xyz(p, self.rotate_deg);
        }
        if !no_trans(self.translate) {
            p = [p[0] + self.translate[0], p[1] + self.translate[1], p[2] + self.translate[2]];
        }
        Some(p)
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
        if !self.in_t_range(x, y) {
            return None;
        }
        let (mut px, mut py) = (x, y);
        if !(self.tscale[0] > 0.9999 && self.tscale[0] < 1.0001 && self.tscale[1] > 0.9999 && self.tscale[1] < 1.0001) {
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

pub fn parse_ini(text: &str, debug: bool) -> HashMap<String, Section> {
    let mut sections: HashMap<String, Section> = HashMap::new();
    let mut current = String::from("global");
    sections.entry(current.clone()).or_default();

    for (lineno, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                current = line[1..end].trim().to_lowercase();
                sections.entry(current.clone()).or_default();
            }
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_lowercase();
            let val = line[eq + 1..].trim().to_string();
            sections.get_mut(&current).unwrap().insert(key, val);
        } else if debug {
            // ponytail: no real logging system, a --debug flag is enough
            // to make silently swallowed lines (missing '=') visible
            eprintln!(
                "[error] Line {} in section [{}] ignored (no '='): {}",
                lineno + 1,
                current,
                line
            );
        }
    }
    sections
}

fn get_str<'a>(sec: &'a Section, key: &str, default: &'a str) -> &'a str {
    sec.get(key).map(|s| s.as_str()).unwrap_or(default)
}

fn get_int(sec: &Section, key: &str, default: i64) -> i64 {
    sec.get(key).and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// Parses "( a , b , c )" -> [a,b,c]. Missing fields become 0.
fn parse_tuple(s: &str) -> Vec<f32> {
    s.trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(',')
        .filter_map(|p| p.trim().parse::<f32>().ok())
        .collect()
}

fn parse_vec3(sec: &Section, key: &str, default: &str) -> Vec3 {
    let raw = get_str(sec, key, default);
    let v = parse_tuple(raw);
    [
        v.first().copied().unwrap_or(0.0),
        v.get(1).copied().unwrap_or(0.0),
        v.get(2).copied().unwrap_or(0.0),
    ]
}

fn parse_vec2(sec: &Section, key: &str, default: &str) -> [f32; 2] {
    let raw = get_str(sec, key, default);
    let v = parse_tuple(raw);
    [v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0)]
}

fn parse_scalar(sec: &Section, key: &str, default: &str) -> f32 {
    let raw = get_str(sec, key, default);
    parse_tuple(raw).first().copied().unwrap_or(0.0)
}

/// Loads all [s0]..[sN-1] sections according to [Global] nTransforms.
pub fn load_transforms(ini_text: &str, debug: bool) -> Result<Vec<Transform>, ParseError> {
    const TRANSFORM_KEYS: &[&str] = &[
    "match", "substitute", "scale", "rotate", "translate", "minimum", "maximum",
    "tscale", "trotate", "ttranslate", "tminimum", "tmaximum", "tbitmap", "position",
    ];
    let sections = parse_ini(ini_text, debug);
    let global = sections
        .get("global")
        .ok_or_else(|| ParseError("no [Global] section found in the INI".into()))?;
    let n = get_int(global, "ntransforms", 0);
    if n <= 0 {
        return Err(ParseError("[Global] nTransforms missing or is 0".into()));
    }

    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let key = format!("s{i}");
        let Some(sec) = sections.get(&key) else { continue };

        let unknown: Vec<&String> = sec.keys().filter(|k| !TRANSFORM_KEYS.contains(&k.as_str())).collect();
        if !unknown.is_empty() {
            eprintln!("Warning: [{key}] contains unknown keywords, ignored. Use --debug for details.");
            if debug {
                for u in &unknown {
                    eprintln!("  [{key}] {u} = {}", sec[*u]);
                }
            }
        }

        let match_pat = get_str(sec, "match", "").to_lowercase();
        if match_pat.is_empty() {
            continue; // deactivated section (corresponds to original: match set to "" = skipped)
        }
        let substitute = get_str(sec, "substitute", "*").to_lowercase();

        let position_raw = parse_vec3(sec, "position", "( -999.0 , -999.0 , -999.0 )");
        let position = if position_raw.iter().any(|c| *c > -998.0) {
            PositionMode::Absolute(position_raw)
        } else {
            PositionMode::LikeVertex
        };

        let tbitmap = sec.get("tbitmap").filter(|s| !s.is_empty()).map(|s| s.to_lowercase());

        out.push(Transform {
            match_pat,
            substitute,
            scale: parse_vec3(sec, "scale", "( 1.0 , 1.0 , 1.0 )"),
            rotate_deg: parse_vec3(sec, "rotate", "( 0.0 , 0.0 , 0.0 )"),
            translate: parse_vec3(sec, "translate", "( 0.0 , 0.0 , 0.0 )"),
            min: parse_vec3(sec, "minimum", "( -999.0 , -999.0 , -999.0 )"),
            max: parse_vec3(sec, "maximum", "( 999.0 , 999.0 , 999.0 )"),
            tscale: parse_vec2(sec, "tscale", "( 1.0 , 1.0 )"),
            trotate_z_deg: parse_scalar(sec, "trotate", "( 0.0 )"),
            ttranslate: parse_vec2(sec, "ttranslate", "( 0.0 , 0.0 )"),
            tmin: parse_vec2(sec, "tminimum", "( -999.0 , -999.0 )"),
            tmax: parse_vec2(sec, "tmaximum", "( 999.0 , 999.0 )"),
            tbitmap,
            position,
        });
    }
    Ok(out)
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
        let transforms = load_transforms(ini, false).unwrap();
        assert_eq!(transforms.len(), 1);
        assert_eq!(transforms[0].scale, [0.72, 0.72, 0.72]);
    }
}