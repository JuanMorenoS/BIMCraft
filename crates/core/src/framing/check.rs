//! The structural check of a framed floor's joists and a framed roof's rafters (`framing_check`): whether lines of the
//! framing type's section and spacing carry the loads over the longest clear span the model gives them, by allowable stress
//! design as the NDS has it, and where they don't, the shallowest depth or the widest spacing that would.
//!
//! Each region a host frames ([`Spanned`]: a floor; each slope of a roof) is checked as one line, simply supported over its
//! longest clear span `L` (in plan, between the faces of what bears it, [`super::Framing::span`]) under a uniform load: the
//! loads per square metre (kPa) times the spacing. A floor's live and dead loads lie on its plan. A roof's live load lies on
//! its plan and its dead load on its slope, so per metre of plan it is `dead / cos θ`; with `w` that load per mm of plan:
//! - Bending: `M = w L² / 8` (the same as the load square to the rafter along its sloped length), `fb = M / S`,
//!   `S = b d² / 6`, against `Fb' = Fb · CF · Cr`.
//! - Shear: `V = w L / 2` (not reduced by `cos θ`, nor taken a depth from the support: on the safe side), `fv = 1.5 V / (b d)`,
//!   against `Fv' = Fv`.
//! - Deflection square to the line along its sloped length `Ls = L / cos θ` under the load's part square to it, `w cos² θ`:
//!   `Δ = 5 (w cos² θ) Ls⁴ / (384 E I)`, `I = b d³ / 12`, under the live load against `Ls / 360` (a floor) or `Ls / 240` (a
//!   roof) and under the whole load against `Ls / 240` (a floor) or `Ls / 180` (a roof). A floor's `θ` is 0.
//!
//! The adjustment factors: `CD` 1.0 (normal duration, on the safe side for a roof's live load), `CM` 1.0 (dry service), `Ct`
//! 1.0, `CL` 1.0 (the compression edge held by the sheathing), the size factor `CF` by depth as the wood's table gives it, and
//! the repetitive member factor `Cr` 1.15 for lines no more than 610 mm (24 in) apart, 1.0 farther. The section is as sawn,
//! `thickness` by the structure layer's depth. Trimmers, headers, the pairs under walls, rims and ridge boards aren't
//! checked: only the lines at the type's spacing.
//!
//! The woods are data (`data/wood.json`), each naming its source; the default loads are a dwelling's usual ones and are
//! checked against nothing: the local code says what they are (in Colombia NSR-10, Título B for loads, Título G for wood).

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Framing, Role};
use crate::commands::bad;
use crate::engine::Session;
use crate::model::{Element, Id, Model};

/// The wood a framing type naming none is checked as.
pub const DEFAULT_WOOD: &str = "douglas_fir_larch_no2";
/// The depths, mm, a check offers in place of one that fails where the type names none: 2x4 to 2x12 as sawn.
pub const DEPTHS: &[f64] = &[89.0, 140.0, 184.0, 235.0, 286.0];
/// The most a load may be, kPa.
pub const MAX_LOAD: f64 = 50.0;
/// The widest spacing a check offers, mm: the most a framing type may have.
const MAX_SPACING: f64 = 3000.0;
/// Lines no farther apart than this, mm (24 in), share their load: the repetitive member factor.
const REPETITIVE: f64 = 610.0;
const CR: f64 = 1.15;
const PSI: f64 = 0.006_894_757_293_168;

/// Loads on a floor or a roof, kPa: `live` (a roof's on its plan, its live or snow load), `dead` (a roof's on its slope).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loads {
    pub live: f64,
    pub dead: f64,
}

/// What a region is framed as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    Floor,
    Roof,
}

impl Use {
    /// The default loads: a floor's 1.9 kPa (40 psf) live and 0.5 kPa dead, a roof's 1.0 kPa live or snow and 0.5 kPa dead.
    pub fn loads(self) -> Loads {
        match self {
            Use::Floor => Loads { live: 1.9, dead: 0.5 },
            Use::Roof => Loads { live: 1.0, dead: 0.5 },
        }
    }

    /// The deflection limits as spans over them: under the live load, and under the whole load.
    fn limits(self) -> [f64; 2] {
        match self {
            Use::Floor => [360.0, 240.0],
            Use::Roof => [240.0, 180.0],
        }
    }

    fn of(role: Role) -> Use {
        if role == Role::Rafter { Use::Roof } else { Use::Floor }
    }
}

/// A region of a host's framing as it is checked: its name (`floor`, `slope <i>`), what its lines are, their longest clear
/// span in plan (mm), how steep they are (degrees) and their section as sawn, [thickness, depth square to the slope].
#[derive(Clone, Debug, PartialEq)]
pub struct Spanned {
    pub region: String,
    pub role: Role,
    pub span: f64,
    pub slope: f64,
    pub section: [f64; 2],
}

/// A row of a wood's table: up to a depth (mm; none, any deeper), a value.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    #[serde(default)]
    up_to: Option<f64>,
    #[serde(default)]
    psi: Option<f64>,
    #[serde(default)]
    cf: Option<f64>,
}

/// A species and grade's reference design values, as `data/wood.json` holds them (psi).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wood {
    pub key: String,
    pub name: String,
    pub source: String,
    /// Values from no standard, to be replaced.
    #[serde(default)]
    pub assumed: bool,
    fb: Vec<Row>,
    #[serde(default)]
    size_factor: Vec<Row>,
    fv: f64,
    e: f64,
}

/// The first row holding `depth`, else the last.
fn row(rows: &[Row], depth: f64) -> Option<&Row> {
    rows.iter().find(|r| r.up_to.is_none_or(|u| depth <= u + 1e-6)).or(rows.last())
}

impl Wood {
    /// The size factor at `depth`: 1.0 where its table has none.
    pub fn cf(&self, depth: f64) -> f64 {
        row(&self.size_factor, depth).and_then(|r| r.cf).unwrap_or(1.0)
    }

    /// Bending at `depth`, MPa: its reference value times the size factor.
    pub fn fb(&self, depth: f64) -> f64 {
        row(&self.fb, depth).and_then(|r| r.psi).unwrap_or(0.0) * self.cf(depth) * PSI
    }

    /// Shear parallel to grain, MPa.
    pub fn fv(&self) -> f64 {
        self.fv * PSI
    }

    /// Modulus of elasticity, MPa.
    pub fn e(&self) -> f64 {
        self.e * PSI
    }

    fn json(&self, given: bool) -> Value {
        json!({"key": self.key, "name": self.name, "source": self.source, "assumed": self.assumed, "given": given})
    }
}

#[derive(Deserialize)]
struct Data {
    woods: Vec<Wood>,
}

/// The woods of `data/wood.json`.
pub fn woods() -> &'static [Wood] {
    static WOODS: OnceLock<Vec<Wood>> = OnceLock::new();
    WOODS.get_or_init(|| serde_json::from_str::<Data>(include_str!("../../data/wood.json")).map(|d| d.woods).unwrap_or_default())
}

/// The wood keyed `key`.
pub fn wood(key: &str) -> Option<&'static Wood> {
    woods().iter().find(|w| w.key == key)
}

/// The woods' keys, for a message.
pub fn wood_keys() -> String {
    woods().iter().map(|w| w.key.as_str()).collect::<Vec<_>>().join(", ")
}

/// One line checked: its section as sawn [b, d], its spacing and clear span in plan (mm), how steep (degrees), and what it
/// frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Case {
    pub section: [f64; 2],
    pub spacing: f64,
    pub span: f64,
    pub slope: f64,
    pub kind: Use,
}

/// What a check found against what is allowed (MPa or mm), and their ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ratio {
    pub value: f64,
    pub allowed: f64,
    pub ratio: f64,
}

impl Ratio {
    fn new(value: f64, allowed: f64) -> Ratio {
        let ratio = if value <= 0.0 {
            0.0
        } else if allowed > 0.0 {
            value / allowed
        } else {
            f64::INFINITY
        };
        Ratio { value, allowed, ratio }
    }

    pub fn ok(&self) -> bool {
        self.ratio <= 1.0 + 1e-9
    }

    fn json(&self) -> Value {
        json!({"value": round(self.value, 1e3), "allowed": round(self.allowed, 1e3), "ratio": round(self.ratio, 1e4), "ok": self.ok()})
    }
}

/// A line's checks: bending and shear stress (MPa), deflection under the live load and the whole load (mm); the size and
/// repetitive member factors taken; its sloped length (mm).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outcome {
    pub bending: Ratio,
    pub shear: Ratio,
    pub live: Ratio,
    pub total: Ratio,
    pub cf: f64,
    pub cr: f64,
    pub length: f64,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.bending.ok() && self.shear.ok() && self.live.ok() && self.total.ok()
    }

    fn worst(&self) -> f64 {
        [self.bending.ratio, self.shear.ratio, self.live.ratio, self.total.ratio].into_iter().fold(0.0, f64::max)
    }
}

/// Line `c` of wood `w` under `loads`, as the module says.
pub fn check(c: &Case, w: &Wood, loads: Loads) -> Outcome {
    let [b, d] = c.section;
    let cos = c.slope.to_radians().cos().max(1e-6);
    // Per mm of plan, N/mm: kPa is 1e-3 N/mm².
    let live = loads.live * 1e-3 * c.spacing;
    let all = (loads.live + loads.dead / cos) * 1e-3 * c.spacing;
    let l = c.span;
    let (s, i) = (b * d * d / 6.0, b * d.powi(3) / 12.0);
    let cf = w.cf(d);
    let cr = if c.spacing <= REPETITIVE + 1e-9 { CR } else { 1.0 };
    let bending = Ratio::new(all * l * l / 8.0 / s, w.fb(d) * cr);
    let shear = Ratio::new(1.5 * (all * l / 2.0) / (b * d), w.fv());
    let length = l / cos;
    let sag = |q: f64| 5.0 * q * cos * cos * length.powi(4) / (384.0 * w.e() * i);
    let [ll, lt] = c.kind.limits();
    Outcome { bending, shear, live: Ratio::new(sag(live), length / ll), total: Ratio::new(sag(all), length / lt), cf, cr, length }
}

/// The shallowest of `depths` at which line `c` passes: none where none does.
pub fn depth(c: &Case, w: &Wood, loads: Loads, depths: &[f64]) -> Option<f64> {
    depths.iter().copied().find(|d| check(&Case { section: [c.section[0], *d], ..*c }, w, loads).ok())
}

/// The widest spacing, to the millimetre and up to 3000, at which line `c` passes at its depth: none narrower than twice
/// its thickness. Every check grows with the spacing, as its load does, but bending also loses `Cr` past 610.
pub fn spacing(c: &Case, w: &Wood, loads: Loads) -> Option<f64> {
    let at = |s: f64| check(&Case { spacing: s, ..*c }, w, loads);
    let widest = |s: f64| {
        let worst = at(s).worst();
        if worst <= 0.0 { MAX_SPACING } else { s / worst }
    };
    // With Cr, as far as 610; without it, past that.
    let near = widest(REPETITIVE).min(REPETITIVE);
    let far = widest(REPETITIVE + 1.0);
    let mut best = if far > REPETITIVE { far } else { near };
    best = best.min(MAX_SPACING).floor();
    // The rounding of a ratio near 1 may leave the floor a hair over: step back to one that passes.
    while best > 0.0 && !at(best).ok() {
        best -= 1.0;
    }
    (best >= 2.0 * c.section[0] && best.is_finite()).then_some(best)
}

fn round(x: f64, by: f64) -> f64 {
    if x.is_finite() { (x * by).round() / by } else { x }
}

/// What a command gives in place of a type's: a wood, live and dead loads.
#[derive(Default)]
pub(crate) struct Over {
    pub wood: Option<&'static Wood>,
    pub live: Option<f64>,
    pub dead: Option<f64>,
}

/// One host checked: its JSON, its warnings (prefixed with `who`) and whether all of it passes.
pub(crate) fn host(m: &Model, id: Id, f: &Framing, over: &Over) -> (Value, Vec<String>, bool) {
    let who = super::named(m, id);
    let t = m.types.framing.get(&f.type_name).cloned().unwrap_or_default();
    let given = over.wood.is_some() || t.wood.is_some();
    let w = over.wood.or_else(|| t.wood.as_deref().and_then(wood)).or_else(|| wood(DEFAULT_WOOD));
    let mut notes = Vec::new();
    let mut warnings = Vec::new();
    let mut regions = Vec::new();
    let mut ok = true;
    let Some(w) = w else {
        return (
            json!({"id": id, "host": f.host, "type": f.type_name, "regions": [], "ok": false}),
            vec![format!("{who}: no wood to check it as")],
            false,
        );
    };
    if !given {
        notes.push(format!(
            "the framing type `{}` names no `wood`: checked as {} ({}); set `wood` to the lumber bought (one of {})",
            f.type_name,
            w.name,
            w.source,
            wood_keys()
        ));
    }
    if w.assumed {
        notes.push(format!("{}'s values are an assumption, not a standard's: {}", w.name, w.source));
    }
    let depths = if t.depths.is_empty() { DEPTHS.to_vec() } else { t.depths.clone() };
    let mut defaults = Vec::new();
    for g in &f.regions {
        let kind = Use::of(g.role);
        let own = match kind {
            Use::Floor => t.floor_loads,
            Use::Roof => t.roof_loads,
        };
        let base = own.unwrap_or(kind.loads());
        let loads = Loads { live: over.live.unwrap_or(base.live), dead: over.dead.unwrap_or(base.dead) };
        if own.is_none() && (over.live.is_none() || over.dead.is_none()) && !defaults.contains(&kind) {
            defaults.push(kind);
        }
        let c = Case { section: g.section, spacing: t.spacing, span: g.span, slope: g.slope, kind };
        let o = check(&c, w, loads);
        let mut v = json!({
            "region": g.region, "role": g.role, "span": round(g.span, 100.0), "slope": round(g.slope, 100.0),
            "length": round(o.length, 100.0), "section": g.section, "spacing": t.spacing, "loads": {"live": loads.live, "dead": loads.dead},
            "factors": {"cd": 1.0, "cm": 1.0, "ct": 1.0, "cl": 1.0, "cf": o.cf, "cr": o.cr},
            "bending": o.bending.json(), "shear": o.shear.json(), "deflection_live": o.live.json(), "deflection_total": o.total.json(),
            "ok": o.ok(),
        });
        if !o.ok() {
            ok = false;
            let d = depth(&c, w, loads, &depths);
            let sp = spacing(&c, w, loads);
            v["suggest"] = json!({"depth": d, "spacing": sp});
            warnings.push(warning(&who, g, &c, &o, w, loads, (d, sp), depths.last().copied().unwrap_or(0.0)));
        }
        regions.push(v);
    }
    for kind in defaults {
        let (what, l) = match kind {
            Use::Floor => ("a floor's", Use::Floor.loads()),
            Use::Roof => ("a roof's (live on plan, dead on the slope)", Use::Roof.loads()),
        };
        notes.push(format!(
            "the loads are the defaults for {what}, {} kPa live and {} kPa dead: check them against the local code (in Colombia \
             NSR-10, Título B for loads and Título G for wood) and set the type's `{}_loads` {{live, dead}}",
            l.live,
            l.dead,
            if kind == Use::Floor { "floor" } else { "roof" }
        ));
    }
    let v = json!({
        "id": id, "host": f.host, "type": f.type_name, "wood": w.json(given), "regions": regions, "ok": ok, "notes": notes,
    });
    (v, warnings, ok)
}

/// A failing line's warning: what it is, where it fails and by how much, and what would pass.
#[allow(clippy::too_many_arguments)]
fn warning(
    who: &str,
    g: &Spanned,
    c: &Case,
    o: &Outcome,
    w: &Wood,
    loads: Loads,
    (d, sp): (Option<f64>, Option<f64>),
    deepest: f64,
) -> String {
    let slope = if g.role == Role::Rafter { format!("{}: ", g.region) } else { String::new() };
    let lines = if g.role == Role::Rafter { "rafters" } else { "joists" };
    let along = if c.slope > 1e-9 { format!(" in plan ({:.0} mm along the {:.0}° slope)", o.length, c.slope) } else { String::new() };
    let failing: Vec<String> =
        [("bending", o.bending), ("shear", o.shear), ("live-load deflection", o.live), ("total deflection", o.total)]
            .into_iter()
            .filter(|(_, r)| !r.ok())
            .map(|(what, r)| format!("{what} at {:.0} %", r.ratio * 100.0))
            .collect();
    let failing = match failing.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let depth = match d {
        Some(d) => format!("a depth of {d:.0} mm passes at this spacing"),
        None => format!("no depth up to {deepest:.0} mm passes at this spacing"),
    };
    let spacing = match sp {
        Some(s) => format!("at this depth they pass {s:.0} mm apart"),
        None => "no spacing passes at this depth".into(),
    };
    format!(
        "{who}: {slope}its {lines}, {:.0} x {:.0} mm at {:.0} mm over a clear span of {:.0} mm{along}, don't carry {} kPa live and {} \
         kPa dead as {}: {failing} of what it allows; {depth}, and {spacing}; or put a beam or a wall under them",
        c.section[0], c.section[1], c.spacing, c.span, loads.live, loads.dead, w.name
    )
}

/// The warnings of framed floors' and roofs' lines that fail, with the type's own wood and loads: for `frame`'s reply.
pub(crate) fn warnings(m: &Model, id: Id, f: &Framing) -> Vec<String> {
    if f.regions.is_empty() {
        return Vec::new();
    }
    host(m, id, f, &Over::default()).1
}

/// A load given to `framing_check`, kPa.
fn load(c: &str, p: &Value, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|x| x.is_finite() && (0.0..=MAX_LOAD).contains(x))
            .map(Some)
            .ok_or_else(|| bad(c, format!("`{key}` is a load in kPa, from 0 to {MAX_LOAD}, got {v}"))),
    }
}

/// `framing_check`: framed floors' joists and roofs' rafters checked for their loads, read and never built.
pub(crate) fn command(s: &mut Session, p: &Value) -> Result<Value, String> {
    let c = "framing_check";
    crate::commands::only(c, p, &["id", "ids", "wood", "live", "dead"])?;
    let m = &s.model;
    let all = matches!(p.get("id"), None | Some(Value::Null)) && matches!(p.get("ids"), None | Some(Value::Null));
    let ids: Vec<Id> = if all {
        m.elements
            .iter()
            .filter(|(_, e)| match e {
                Element::Floor(f) => f.framing.is_some(),
                Element::Roof(r) => r.framing.is_some(),
                _ => false,
            })
            .map(|(id, _)| *id)
            .collect()
    } else {
        crate::commands::ids_param(c, m, p)?
    };
    if let Some((id, e)) =
        ids.iter().filter_map(|id| Some((*id, m.elements.get(*id)?))).find(|(_, e)| !matches!(e, Element::Floor(_) | Element::Roof(_)))
    {
        return Err(bad(
            c,
            format!(
                "{} {id} isn't a floor or a roof: only floors and roofs are checked (a wall's studs and headers aren't yet)",
                e.category()
            ),
        ));
    }
    let mut over = Over { live: load(c, p, "live")?, dead: load(c, p, "dead")?, wood: None };
    if let Some(k) = crate::commands::string(c, p, "wood")? {
        over.wood = Some(wood(&k).ok_or_else(|| bad(c, format!("there is no wood `{k}`; there are: {}", wood_keys())))?);
    }
    let (mut hosts, mut warnings, mut ok) = (Vec::new(), Vec::new(), true);
    for id in ids {
        let who = super::named(m, id);
        let f = super::of(m, id)
            .map_err(|e| bad(c, format!("{who}: {e}")))?
            .ok_or_else(|| bad(c, format!("{who} isn't framed: `frame` it")))?;
        let (v, w, good) = host(m, id, &f, &over);
        hosts.push(v);
        warnings.extend(w);
        ok &= good;
    }
    Ok(json!({"units": {"length": "mm", "stress": "MPa", "load": "kPa"}, "hosts": hosts, "ok": ok, "warnings": warnings}))
}
