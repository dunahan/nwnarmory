// Portierung der Transform-/INI-Logik aus Transform.cpp + ini2.cpp.
//
// Bewusste Vereinfachungen gegenüber dem Original (ponytail: YAGNI):
// - Keine generische CMatrix-Klasse mit Referenzzählung; Rotation direkt
//   als drei Achsen-Rotationen auf [f32; 3] angewendet (Ref.-Zählung war
//   nur wegen manueller C++-Speicherverwaltung nötig, in Rust überflüssig).
// - `position=(x,y,z)` wird gemäss readme.txt als ABSOLUTE Verschiebung
//   behandelt (ersetzt die Original-Position), nicht als Artefakt der
//   speziellen CMatrix-Multiplikation im Original.
// - wildcmp ist der Standard-Glob-Algorithmus (funktional identisch für
//   alle Muster in den mitgelieferten .ini-Dateien, ohne die Kanten der
//   handgeschriebenen Original-Implementierung zu übernehmen).

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

/// Eine geladene Transform-Sektion ([s0], [s1], ...).
pub struct Transform {
    pub match_pat: String,   // bereits kleingeschrieben
    pub substitute: String,  // bereits kleingeschrieben

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
    pub tbitmap: Option<String>, // Some(name) = tvert-Transform nur für dieses Bitmap

    pub position: PositionMode,
}

#[derive(Clone, Copy)]
pub enum PositionMode {
    /// Kein position=(...) angegeben: Position wird wie ein normaler Vertex
    /// behandelt (Scale/Rotate/Translate, falls innerhalb min/max).
    LikeVertex,
    /// position=(x,y,z) angegeben: absolute Verschiebung (ersetzt die
    /// Original-Position), siehe readme.txt.
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

    /// Wendet Scale/Rotate/Translate auf einen Vertex an, sofern nicht
    /// "just copy" und innerhalb min/max. Gibt None zurück wenn die Zeile
    /// unverändert (Originaltext) übernommen werden soll.
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

    /// Wendet TScale/TRotate(nur Z)/TTranslate auf eine Texturkoordinate an.
    /// `last_bitmap` ist das zuletzt gesehene `bitmap`-Statement (klein).
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

/// Wendet Rotation in Grad an: X- und Y-Achse negiert (Max ist
/// linkshändig, siehe Original-Kommentar in Transform.cpp), Reihenfolge
/// X dann Y dann Z, entsprechend der Original-Multiplikationsreihenfolge.
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
// INI-Parsing (Ersatz fuer ini2.cpp / GetPrivateProfileString)
// ---------------------------------------------------------------------

type Section = HashMap<String, String>;

pub fn parse_ini(text: &str) -> HashMap<String, Section> {
    let mut sections: HashMap<String, Section> = HashMap::new();
    let mut current = String::from("global");
    sections.entry(current.clone()).or_default();

    for raw_line in text.lines() {
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

/// Parst "( a , b , c )" -> [a,b,c]. Fehlende Felder werden 0.
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

/// Laedt alle [s0]..[sN-1] Sektionen gemaess [Global] nTransforms.
pub fn load_transforms(ini_text: &str) -> Result<Vec<Transform>, ParseError> {
    let sections = parse_ini(ini_text);
    let global = sections
        .get("global")
        .ok_or_else(|| ParseError("keine [Global]-Sektion in der INI gefunden".into()))?;
    let n = get_int(global, "ntransforms", 0);
    if n <= 0 {
        return Err(ParseError("[Global] nTransforms fehlt oder ist 0".into()));
    }

    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let key = format!("s{i}");
        let Some(sec) = sections.get(&key) else { continue };

        let match_pat = get_str(sec, "match", "").to_lowercase();
        if match_pat.is_empty() {
            continue; // deaktivierte Sektion (entspricht Original: match auf "" gesetzt = uebersprungen)
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
// Wildcard-Matching (Ersatz fuer wildcmp in IO.cpp)
// ---------------------------------------------------------------------

/// Standard-Glob-Matching mit `*` und `?`. Erwartet bereits
/// kleingeschriebene Strings (Original vergleicht ebenfalls case-insensitiv).
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

/// Baut den Zielmodellnamen aus `old_name` (klein) und `substitute`-Muster.
/// Entspricht CIO::doSubstitute in IO.cpp.
pub fn build_substitute(old_name: &str, subst: &str) -> String {
    let mut out: Vec<char> = old_name.chars().collect();
    let mut idx = 0usize;
    for c in subst.chars() {
        match c {
            '?' => idx += 1,
            '*' => idx = out.len(),
            '\\' | '/' | ':' | '"' | '<' | '>' | '|' => { /* ungueltiges Wildcard-Zeichen, ueberspringen */ }
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
        // "pm" bleibt (2x '?'), 'a' ersetzt die Rassen-Stelle (Original-Ziffer '0'),
        // '*' uebernimmt den Rest inkl. erhaltener Phaenotyp-Ziffer '1'.
        // Ergibt "pma1_belt001" (Player, maennlich, Halfling(a), Phaenotyp 1) --
        // konsistent mit der echten NWN-Namenskonvention p<Geschlecht><Rasse><Phaenotyp>.
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
        let transforms = load_transforms(ini).unwrap();
        assert_eq!(transforms.len(), 1);
        assert_eq!(transforms[0].scale, [0.72, 0.72, 0.72]);
    }
}
