//! The structural check of framed floors' joists and roofs' rafters (`framing/check.rs`): hand-calculated cases, then the
//! `framing_check` command and `frame`'s reply driven through `Session::execute`.

use serde_json::{Value, json};

use crate::engine::Session;
use crate::framing::check::{self, Case, Loads, Use};

fn run(s: &mut Session, c: &str, p: Value) -> Value {
    s.execute(c, &p).unwrap_or_else(|e| panic!("{c} {p}: {e}"))
}

fn first(v: &Value) -> u64 {
    v["ids"][0].as_u64().or(v["id"].as_u64()).unwrap_or_else(|| panic!("no id in {v}"))
}

fn near(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() <= 0.002 * b.abs().max(1.0), "{what}: {a} isn't {b}");
}

fn df() -> &'static check::Wood {
    check::wood("douglas_fir_larch_no2").unwrap_or_else(|| panic!("no Douglas Fir-Larch No. 2 in data/wood.json"))
}

fn joist(span: f64) -> Case {
    Case { section: [38.0, 235.0], spacing: 406.4, span, slope: 0.0, kind: Use::Floor }
}

#[test]
fn the_wood_table_reads_and_names_its_sources() {
    let woods = check::woods();
    assert!(woods.len() >= 3, "{woods:?}");
    for w in woods {
        assert!(!w.source.is_empty() && !w.name.is_empty(), "{} names its source", w.key);
    }
    let sp = check::wood("southern_pine_no2").unwrap_or_else(|| panic!("no Southern Pine No. 2"));
    // NDS 2018 Supplement Table 4B (the June 2013 values): Fb by width, the size factor in them; Fv 175, E 1,400,000 psi.
    let psi = 0.00689475729;
    for (depth, fb) in [(89.0, 1100.0), (140.0, 1000.0), (184.0, 925.0), (235.0, 800.0), (286.0, 750.0)] {
        near(sp.fb(depth), fb * psi, &format!("Southern Pine No. 2 Fb at {depth}"));
    }
    near(sp.fv(), 175.0 * psi, "Southern Pine No. 2 Fv");
    near(sp.e(), 1_400_000.0 * psi, "Southern Pine No. 2 E");
    // Table 4A: Fb 900 psi times the size factor by width (1.1 for a 2x10, 1.0 for a 2x12).
    near(df().fb(235.0), 900.0 * 1.1 * psi, "Douglas Fir-Larch No. 2 Fb of a 2x10");
    near(df().fb(286.0), 900.0 * psi, "Douglas Fir-Larch No. 2 Fb of a 2x12");
    near(df().fv(), 180.0 * psi, "Douglas Fir-Larch No. 2 Fv");
    near(df().e(), 1_600_000.0 * psi, "Douglas Fir-Larch No. 2 E");
    let assumed: Vec<&str> = woods.iter().filter(|w| w.assumed).map(|w| w.key.as_str()).collect();
    assert_eq!(assumed, ["plantation_pine_assumed"], "the plantation pine is the one assumption");
}

/// A 2x10 Douglas Fir-Larch No. 2 joist 16 inches apart over 4.0 m, 1.9 kPa live and 0.5 kPa dead: by hand, w = 2.4 kPa x
/// 0.4064 m = 0.9754 N/mm; M = wL²/8 = 1.951 kN·m over S = 38 x 235² / 6 = 349 758 mm³ is 5.577 MPa against Fb' = 900 psi x
/// 1.1 (CF) x 1.15 (Cr) = 7.850 MPa; V = wL/2 = 1951 N, 1.5V/bd = 0.328 MPa against 180 psi = 1.241 MPa; under the live
/// load 0.7722 N/mm, 5wL⁴/384EI = 5.68 mm against L/360 = 11.1, and under all of it 7.17 against L/240 = 16.7.
#[test]
fn a_2x10_douglas_fir_joist_at_406_over_4_m_passes() {
    let o = check::check(&joist(4000.0), df(), Use::Floor.loads());
    near(o.bending.value, 5.5773, "bending stress");
    near(o.bending.allowed, 7.8497, "allowed bending stress");
    near(o.bending.ratio, 0.7105, "bending");
    near(o.shear.ratio, 0.2640, "shear");
    near(o.live.value, 5.6773, "live-load deflection");
    near(o.live.allowed, 4000.0 / 360.0, "live-load deflection limit");
    near(o.total.value, 7.1713, "total deflection");
    near(o.total.allowed, 4000.0 / 240.0, "total deflection limit");
    assert_eq!((o.cf, o.cr), (1.1, 1.15));
    assert!(o.ok(), "{o:?}");
}

/// The same joist over 6.0 m: bending 160 %, live-load deflection 172 % (28.7 mm against 16.7), total 145 %. A 2x12 at 406
/// still fails bending (8.47 MPa against 900 psi x 1.0 x 1.15 = 7.14), so no depth up to 286 passes at this spacing; at this
/// depth the joists pass 235 mm apart (406.4 / 1.724), the live-load deflection governing.
#[test]
fn the_same_joist_over_6_m_fails_deflection_and_bending_and_says_what_would_pass() {
    let c = joist(6000.0);
    let o = check::check(&c, df(), Use::Floor.loads());
    near(o.live.value, 28.7413, "live-load deflection");
    near(o.live.ratio, 1.7245, "live-load deflection");
    near(o.total.ratio, 1.4522, "total deflection");
    near(o.bending.ratio, 1.5987, "bending");
    assert!(!o.ok() && !o.live.ok() && !o.bending.ok() && o.shear.ok(), "{o:?}");
    let deeper = check::check(&Case { section: [38.0, 286.0], ..c }, df(), Use::Floor.loads());
    near(deeper.bending.ratio, 1.1873, "a 2x12's bending");
    near(deeper.live.ratio, 0.9567, "a 2x12's live-load deflection");
    assert_eq!(check::depth(&c, df(), Use::Floor.loads(), check::DEPTHS), None);
    assert_eq!(check::spacing(&c, df(), Use::Floor.loads()), Some(235.0));
    // Over 5 m a 2x12 passes at 406.
    assert_eq!(check::depth(&joist(5000.0), df(), Use::Floor.loads(), check::DEPTHS), Some(286.0));
}

/// A 2x8 Douglas Fir-Larch No. 2 rafter 24 inches apart on a 30° slope over 3.0 m in plan, 1.0 kPa live on plan and 0.5 kPa
/// dead on the slope: by hand, w = (1.0 + 0.5 / cos 30°) x 0.6096 = 0.9616 N/mm of plan; M = wLh²/8 = 1.082 kN·m, 5.045 MPa
/// against 900 x 1.2 x 1.15 psi = 8.563 MPa. Deflection square to the rafter along its sloped length Ls = 3464 mm, under
/// w cos²30°: live 0.4572 N/mm, 3.94 mm against Ls/240 = 14.4; total 6.21 against Ls/180 = 19.2.
#[test]
fn a_2x8_rafter_at_610_on_a_30_degree_slope_over_3_m_in_plan_passes() {
    let c = Case { section: [38.0, 184.0], spacing: 609.6, span: 3000.0, slope: 30.0, kind: Use::Roof };
    let o = check::check(&c, df(), Use::Roof.loads());
    near(o.length, 3464.10, "sloped length");
    near(o.bending.ratio, 0.5891, "bending");
    near(o.shear.ratio, 0.2493, "shear");
    near(o.live.value, 3.9392, "live-load deflection");
    near(o.live.allowed, 3464.10 / 240.0, "live-load deflection limit");
    near(o.total.value, 6.2136, "total deflection");
    near(o.total.allowed, 3464.10 / 180.0, "total deflection limit");
    assert!(o.ok(), "{o:?}");
    // Over 5 m in plan it fails bending (164 %) and deflection; the depth that passes is the first that does, and the one
    // before it doesn't.
    let long = Case { span: 5000.0, ..c };
    let o = check::check(&long, df(), Use::Roof.loads());
    near(o.bending.ratio, 1.6365, "bending over 5 m");
    near(o.total.ratio, 1.4948, "total deflection over 5 m");
    let d = check::depth(&long, df(), Use::Roof.loads(), check::DEPTHS).unwrap_or_else(|| panic!("a depth passes"));
    assert!(check::check(&Case { section: [38.0, d], ..long }, df(), Use::Roof.loads()).ok());
    let before = check::DEPTHS.iter().copied().rfind(|x| *x < d).unwrap_or(0.0);
    assert!(!check::check(&Case { section: [38.0, before], ..long }, df(), Use::Roof.loads()).ok(), "{before}");
    let sp = check::spacing(&long, df(), Use::Roof.loads()).unwrap_or_else(|| panic!("a spacing passes"));
    assert!(check::check(&Case { spacing: sp, ..long }, df(), Use::Roof.loads()).ok(), "{sp}");
    assert!(!check::check(&Case { spacing: sp + 1.0, ..long }, df(), Use::Roof.loads()).ok(), "{sp}");
}

#[test]
fn past_24_inches_apart_the_repetitive_member_factor_is_lost() {
    let c = Case { spacing: 610.0, ..joist(3000.0) };
    assert_eq!(check::check(&c, df(), Use::Floor.loads()).cr, 1.15);
    assert_eq!(check::check(&Case { spacing: 611.0, ..c }, df(), Use::Floor.loads()).cr, 1.0);
    // The spacing search knows: a span that passes wide apart reaches past 610 with Cr 1.0.
    let short = joist(3000.0);
    let sp = check::spacing(&short, df(), Use::Floor.loads()).unwrap_or(0.0);
    assert!(sp > 610.0, "{sp}");
    assert!(check::check(&Case { spacing: sp, ..short }, df(), Use::Floor.loads()).ok());
    assert!(!check::check(&Case { spacing: sp + 1.0, ..short }, df(), Use::Floor.loads()).ok());
    // No load, nothing fails: any spacing up to the type's most.
    let none = Loads { live: 0.0, dead: 0.0 };
    assert!(check::check(&joist(6000.0), df(), none).ok());
}

/// A floor type with a 235 mm structure layer (2x10 joists), and a framed floor `width` across its joists (north) and 3000
/// along them, its rims on the north and south edges: the joists' clear span is `width` less the two rims.
fn floor(s: &mut Session, width: f64, framing: Value) -> u64 {
    run(
        s,
        "type",
        json!({"category": "floor", "name": "2x10 Floor", "layers": [
            {"function": "substrate", "material": "Timber", "thickness": 18},
            {"function": "structure", "material": "Timber", "thickness": 235}]}),
    );
    let mut t = json!({"category": "framing", "name": "Joists"});
    if let (Some(o), Some(e)) = (t.as_object_mut(), framing.as_object()) {
        o.extend(e.clone());
    }
    run(s, "type", t);
    let id = run(s, "floor", json!({"boundary": [[0, 0], [3000, 0], [3000, width], [0, width]], "type": "2x10 Floor"}))["id"]
        .as_u64()
        .unwrap_or(0);
    run(s, "frame", json!({"id": id, "type": "Joists", "angle": 90}));
    id
}

#[test]
fn framing_check_says_a_floor_of_2x10s_over_4_m_carries_its_loads() {
    let mut s = Session::new();
    let id = floor(&mut s, 4076.0, json!({}));
    let before = s.model.to_json();
    let r = run(&mut s, "framing_check", json!({"id": id}));
    assert_eq!(s.model.to_json(), before, "a read changes nothing");
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(r["warnings"], json!([]), "{r}");
    let h = &r["hosts"][0];
    assert_eq!(h["id"], json!(id));
    assert_eq!(h["wood"]["key"], json!("douglas_fir_larch_no2"), "the default wood");
    assert_eq!(h["wood"]["given"], json!(false));
    let g = &h["regions"][0];
    assert_eq!(g["region"], json!("floor"));
    assert_eq!(g["role"], json!("joist"));
    near(g["span"].as_f64().unwrap_or(0.0), 4000.0, "span");
    assert_eq!(g["section"], json!([38.0, 235.0]));
    assert_eq!(g["loads"], json!({"live": 1.9, "dead": 0.5}));
    near(g["bending"]["ratio"].as_f64().unwrap_or(0.0), 0.7105, "bending");
    near(g["deflection_live"]["ratio"].as_f64().unwrap_or(0.0), 0.5110, "live-load deflection");
    assert_eq!(g["ok"], json!(true));
    assert!(g.get("suggest").is_none(), "{g}");
    // The notes say what was assumed.
    assert!(h["notes"].to_string().contains("names no `wood`"), "{h}");
}

#[test]
fn framing_check_and_frame_warn_of_a_floor_of_2x10s_over_6_m() {
    let mut s = Session::new();
    let id = floor(&mut s, 6076.0, json!({}));
    let r = run(&mut s, "framing_check", json!({"ids": [id]}));
    assert_eq!(r["ok"], json!(false), "{r}");
    let g = &r["hosts"][0]["regions"][0];
    assert_eq!(g["deflection_live"]["ok"], json!(false));
    near(g["deflection_live"]["ratio"].as_f64().unwrap_or(0.0), 1.7245, "live-load deflection");
    assert_eq!(g["suggest"], json!({"depth": null, "spacing": 235.0}), "{g}");
    let w = r["warnings"].as_array().cloned().unwrap_or_default();
    assert_eq!(w.len(), 1, "{r}");
    let w = w[0].as_str().unwrap_or("");
    assert!(w.starts_with(&format!("floor {id}: its joists, 38 x 235 mm at 406 mm over a clear span of 6000 mm")), "{w}");
    assert!(w.contains("bending at 160 %") && w.contains("live-load deflection at 172 %"), "{w}");
    assert!(w.contains("235 mm apart"), "{w}");
    // `frame` says so too, apart from what couldn't be framed.
    let f = run(&mut s, "frame", json!({"id": id, "type": "Joists"}));
    assert_eq!(f["structure"], json!([w]), "{f}");
    // Southern Pine No. 2 and a lighter live load, given to the command: a 2x10 at 800 psi still fails at 6 m.
    let r = run(&mut s, "framing_check", json!({"id": id, "wood": "southern_pine_no2", "live": 1.5, "dead": 0.4}));
    let g = &r["hosts"][0]["regions"][0];
    assert_eq!(g["loads"], json!({"live": 1.5, "dead": 0.4}));
    assert_eq!(r["hosts"][0]["wood"]["key"], json!("southern_pine_no2"));
    assert_eq!(r["ok"], json!(false));
}

#[test]
fn a_framing_types_wood_loads_and_depths_are_its_own_and_checked() {
    let mut s = Session::new();
    // Over 5 m with the floor's default loads 2x10s fail; the type's own depths offer a 2x12 (286), and lighter loads pass.
    let id = floor(&mut s, 5076.0, json!({"wood": "douglas_fir_larch_no2", "depths": [235, 286, 337]}));
    let r = run(&mut s, "framing_check", json!({"id": id}));
    assert_eq!(r["hosts"][0]["wood"]["given"], json!(true));
    assert_eq!(r["hosts"][0]["regions"][0]["suggest"]["depth"], json!(286.0), "{r}");
    run(&mut s, "type", json!({"category": "framing", "name": "Joists", "floor_loads": {"live": 0.5, "dead": 0.3}}));
    let r = run(&mut s, "framing_check", json!({"id": id}));
    assert_eq!(r["hosts"][0]["regions"][0]["loads"], json!({"live": 0.5, "dead": 0.3}));
    assert_eq!(r["ok"], json!(true), "{r}");
    // A type saved with them reads back with them; one without writes none of them.
    let t = run(&mut s, "type", json!({"category": "framing", "name": "Plain"}));
    for k in ["wood", "floor_loads", "roof_loads", "depths"] {
        assert!(t["type"].get(k).is_none(), "{k} in {t}");
    }
    // What the type may not hold.
    for (bad, says) in [
        (json!({"wood": "balsa"}), "wood `balsa`"),
        (json!({"floor_loads": {"live": -1, "dead": 0.5}}), "live load"),
        (json!({"roof_loads": {"live": 1, "dead": 1e300}}), "dead load"),
        (json!({"roof_loads": {"live": 1}}), "dead"),
        (json!({"depths": [235, 184]}), "depths"),
        (json!({"depths": [0.5]}), "depth"),
    ] {
        let mut p = json!({"category": "framing", "name": "Joists"});
        p.as_object_mut().into_iter().for_each(|o| o.extend(bad.as_object().cloned().unwrap_or_default()));
        let e = s.execute("type", &p).unwrap_err();
        assert!(e.contains(says), "{bad}: {e}");
    }
}

#[test]
fn framing_check_refuses_what_it_cant_check() {
    let mut s = Session::new();
    let id = floor(&mut s, 4076.0, json!({}));
    let wall = first(&run(&mut s, "wall", json!({"points": [[0, -2000], [4000, -2000]]})));
    let e = s.execute("framing_check", &json!({"id": wall})).unwrap_err();
    assert!(e.contains("only floors and roofs"), "{e}");
    let plain = first(&run(&mut s, "floor", json!({"boundary": [[0, -9000], [3000, -9000], [3000, -6000], [0, -6000]]})));
    let e = s.execute("framing_check", &json!({"id": plain})).unwrap_err();
    assert!(e.contains("isn't framed"), "{e}");
    for (p, says) in [
        (json!({"id": id, "wood": "balsa"}), "no wood `balsa`"),
        (json!({"id": id, "live": -1}), "`live`"),
        (json!({"id": id, "dead": "x"}), "`dead`"),
        (json!({"id": id, "nope": 1}), "nope"),
    ] {
        let e = s.execute("framing_check", &p).unwrap_err();
        assert!(e.contains(says), "{p}: {e}");
    }
    // With no id, every framed floor and roof.
    let r = run(&mut s, "framing_check", json!({}));
    assert_eq!(r["hosts"].as_array().map(Vec::len), Some(1), "{r}");
    // A session that may only read runs it.
    s.read_only = Some("a test");
    assert_eq!(run(&mut s, "framing_check", json!({"id": id}))["ok"], json!(true));
}

/// A gable roof, its eaves along y 0 and y 4000 sloping 30° to a ridge at y 2000, of 2x8 rafters 24 inches apart with no
/// wall under it: each slope's rafters span from the eave to the ridge board, a little under 2000 in plan.
#[test]
fn framing_check_checks_each_slope_of_a_roof_with_its_slope() {
    let mut s = Session::new();
    run(
        &mut s,
        "type",
        json!({"category": "roof", "name": "2x8 Roof", "layers": [{"function": "structure", "material": "Timber", "thickness": 184}]}),
    );
    run(&mut s, "type", json!({"category": "framing", "name": "Rafters", "spacing": 609.6, "rim": false}));
    let id = run(
        &mut s,
        "roof",
        json!({"boundary": [[0, 0], [6000, 0], [6000, 4000], [0, 4000]], "slope": 30, "gables": [1, 3], "overhang": 0,
               "type": "2x8 Roof", "level": "Level 1", "offset": 2400}),
    )["id"]
        .as_u64()
        .unwrap_or(0);
    run(&mut s, "frame", json!({"id": id, "type": "Rafters"}));
    let r = run(&mut s, "framing_check", json!({"id": id}));
    let regions = r["hosts"][0]["regions"].as_array().cloned().unwrap_or_default();
    assert_eq!(regions.len(), 2, "{r}");
    for g in &regions {
        assert!(g["region"].as_str().unwrap_or("").starts_with("slope "), "{g}");
        assert_eq!(g["role"], json!("rafter"));
        near(g["slope"].as_f64().unwrap_or(0.0), 30.0, "slope");
        let span = g["span"].as_f64().unwrap_or(0.0);
        assert!(span > 1900.0 && span < 2000.0, "{g}");
        assert_eq!(g["loads"], json!({"live": 1.0, "dead": 0.5}));
        // As the hand-checked function says for that span.
        let c = Case { section: [38.0, 184.0], spacing: 609.6, span, slope: 30.0, kind: Use::Roof };
        let o = check::check(&c, df(), Use::Roof.loads());
        near(g["bending"]["ratio"].as_f64().unwrap_or(0.0), o.bending.ratio, "bending");
        near(g["length"].as_f64().unwrap_or(0.0), span / 30f64.to_radians().cos(), "sloped length");
        near(g["deflection_total"]["allowed"].as_f64().unwrap_or(0.0), o.length / 180.0, "total deflection limit");
        assert_eq!(g["ok"], json!(true));
    }
}
