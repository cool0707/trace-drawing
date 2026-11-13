//! SVG / DXF 出力。

use crate::doc::{arc_points, Document, Layer, LineType, Shape};
use serde::{Deserialize, Serialize};
use std::fmt::Write;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportOpts {
    /// 出力するレイヤ
    pub line: bool,
    pub hatch: bool,
    pub text: bool,
    pub fill: bool,
    /// 1 画素あたりの出力単位 (mm)。例: 300dpi・縮尺 1/100 の実寸なら 25.4/300*100。
    pub scale: f64,
    /// 文字入力していない文字ブロックを、トレースしたストロークのまま出力する
    pub text_strokes: bool,
}

impl Default for ExportOpts {
    fn default() -> Self {
        ExportOpts {
            line: true,
            hatch: false,
            text: true,
            fill: true,
            scale: 1.0,
            text_strokes: true,
        }
    }
}

impl ExportOpts {
    fn has(&self, l: Layer) -> bool {
        match l {
            Layer::Line => self.line,
            Layer::Hatch => self.hatch,
            Layer::Text => self.text,
            Layer::Fill => self.fill,
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn n(v: f64) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.into()
    }
}

pub fn to_svg(doc: &Document, o: &ExportOpts) -> String {
    let k = if o.scale > 0.0 { o.scale } else { 1.0 };
    let (w, h) = (doc.w as f64 * k, doc.h as f64 * k);
    let sw = (doc.sw as f64 * k * 0.6).max(0.05);
    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{}mm" height="{}mm" viewBox="0 0 {} {}">"#,
        n(w),
        n(h),
        n(w),
        n(h)
    );
    let dash = |lt: LineType| -> String {
        match lt {
            LineType::Solid => String::new(),
            LineType::Dashed => format!(r#" stroke-dasharray="{} {}""#, n(sw * 8.0), n(sw * 4.0)),
            LineType::Chain => format!(
                r#" stroke-dasharray="{} {} {} {}""#,
                n(sw * 24.0),
                n(sw * 4.0),
                n(sw * 2.0),
                n(sw * 4.0)
            ),
        }
    };
    let pt = |q: [f32; 2]| -> String { format!("{},{}", n(q[0] as f64 * k), n(q[1] as f64 * k)) };
    for (layer, name, color) in [
        (Layer::Fill, "FILL", "#000000"),
        (Layer::Hatch, "HATCH", "#808080"),
        (Layer::Line, "LINE", "#000000"),
        (Layer::Text, "TEXT", "#000000"),
    ] {
        if !o.has(layer) {
            continue;
        }
        let fill = if layer == Layer::Fill { color } else { "none" };
        let _ = writeln!(
            s,
            r#"<g id="{name}" fill="{fill}" stroke="{color}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="miter">"#,
            n(sw)
        );
        for e in doc.ents.iter().filter(|e| e.layer == layer) {
            let d = dash(e.lt);
            match &e.shape {
                Shape::Line { a, b } => {
                    let _ = writeln!(
                        s,
                        r#"<line x1="{}" y1="{}" x2="{}" y2="{}"{d}/>"#,
                        n(a[0] as f64 * k),
                        n(a[1] as f64 * k),
                        n(b[0] as f64 * k),
                        n(b[1] as f64 * k)
                    );
                }
                Shape::Poly { pts, closed } => {
                    let tag = if *closed { "polygon" } else { "polyline" };
                    let p: Vec<String> = pts.iter().map(|q| pt(*q)).collect();
                    let _ = writeln!(s, r#"<{tag} points="{}"{d}/>"#, p.join(" "));
                }
                Shape::Circle { c, r } => {
                    let _ = writeln!(
                        s,
                        r#"<circle cx="{}" cy="{}" r="{}"{d}/>"#,
                        n(c[0] as f64 * k),
                        n(c[1] as f64 * k),
                        n(*r as f64 * k)
                    );
                }
                Shape::Ellipse { c, rx, ry, rot } => {
                    let (cx, cy) = (c[0] as f64 * k, c[1] as f64 * k);
                    let _ = writeln!(
                        s,
                        r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" transform="rotate({} {} {})"{d}/>"#,
                        n(cx),
                        n(cy),
                        n(*rx as f64 * k),
                        n(*ry as f64 * k),
                        n(rot.to_degrees() as f64),
                        n(cx),
                        n(cy)
                    );
                }
                Shape::Arc { c, r, a0, sweep } => {
                    let e1 = a0 + sweep;
                    let p0 = [c[0] + r * a0.cos(), c[1] + r * a0.sin()];
                    let p1 = [c[0] + r * e1.cos(), c[1] + r * e1.sin()];
                    let large = if sweep.abs() > std::f32::consts::PI {
                        1
                    } else {
                        0
                    };
                    let dirf = if *sweep > 0.0 { 1 } else { 0 };
                    let _ = writeln!(
                        s,
                        r#"<path d="M{} A{} {} 0 {large} {dirf} {}"{d}/>"#,
                        pt(p0),
                        n(*r as f64 * k),
                        n(*r as f64 * k),
                        pt(p1)
                    );
                }
                Shape::Text {
                    bbox,
                    s: txt,
                    strokes,
                } => {
                    if !txt.is_empty() {
                        let hgt = (bbox[3] - bbox[1]) as f64 * k;
                        let _ = writeln!(
                            s,
                            r#"<text x="{}" y="{}" font-size="{}" fill="{color}" stroke="none" font-family="sans-serif">{}</text>"#,
                            n(bbox[0] as f64 * k),
                            n(bbox[3] as f64 * k),
                            n(hgt),
                            esc(txt)
                        );
                    } else if o.text_strokes {
                        let mut dpath = String::new();
                        for st in strokes {
                            for (i, q) in st.iter().enumerate() {
                                let _ =
                                    write!(dpath, "{}{} ", if i == 0 { "M" } else { "L" }, pt(*q));
                            }
                        }
                        let _ = writeln!(s, r#"<path d="{}"/>"#, dpath.trim_end());
                    }
                }
            }
        }
        s.push_str("</g>\n");
    }
    s.push_str("</svg>\n");
    s
}

/// DXF (R12 / AC1009, Shift_JIS)。Jw_cad・AutoCAD 系のどちらでも読める最大公約数的な形式。
pub fn to_dxf(doc: &Document, o: &ExportOpts) -> Vec<u8> {
    let k = if o.scale > 0.0 { o.scale } else { 1.0 };
    let hgt = doc.h as f64;
    let x = |v: f32| -> f64 { v as f64 * k };
    let y = |v: f32| -> f64 { (hgt - v as f64) * k };
    let mut s = String::new();
    let mut g = |code: i32, val: &str| {
        let _ = write!(s, "{code:>3}\r\n{val}\r\n");
    };
    let f = |v: f64| -> String { format!("{:.4}", v) };
    let unit = (doc.sw as f64 * k).max(0.01);

    g(0, "SECTION");
    g(2, "HEADER");
    g(9, "$ACADVER");
    g(1, "AC1009");
    g(9, "$DWGCODEPAGE");
    g(3, "ANSI_932");
    g(9, "$EXTMIN");
    g(10, "0.0");
    g(20, "0.0");
    g(9, "$EXTMAX");
    g(10, &f(doc.w as f64 * k));
    g(20, &f(hgt * k));
    g(9, "$LTSCALE");
    g(40, "1.0");
    g(0, "ENDSEC");

    g(0, "SECTION");
    g(2, "TABLES");
    g(0, "TABLE");
    g(2, "LTYPE");
    g(70, "3");
    for (name, desc, pat) in [
        ("CONTINUOUS", "Solid line", vec![]),
        ("DASHED", "__ __ __ __", vec![8.0, -4.0]),
        ("CENTER", "____ _ ____ _", vec![24.0, -4.0, 2.0, -4.0]),
    ] {
        g(0, "LTYPE");
        g(2, name);
        g(70, "0");
        g(3, desc);
        g(72, "65");
        g(73, &pat.len().to_string());
        g(
            40,
            &f(pat.iter().map(|v: &f64| v.abs()).sum::<f64>() * unit),
        );
        for v in pat {
            g(49, &f(v * unit));
        }
    }
    g(0, "ENDTAB");
    g(0, "TABLE");
    g(2, "LAYER");
    g(70, "5");
    for (name, color) in [
        ("0", 7),
        ("LINE", 7),
        ("HATCH", 8),
        ("TEXT", 3),
        ("FILL", 5),
    ] {
        g(0, "LAYER");
        g(2, name);
        g(70, "0");
        g(62, &color.to_string());
        g(6, "CONTINUOUS");
    }
    g(0, "ENDTAB");
    g(0, "TABLE");
    g(2, "STYLE");
    g(70, "1");
    g(0, "STYLE");
    g(2, "STANDARD");
    g(70, "0");
    g(40, "0.0");
    g(41, "1.0");
    g(50, "0.0");
    g(71, "0");
    g(42, "2.5");
    g(3, "txt");
    g(4, "");
    g(0, "ENDTAB");
    g(0, "ENDSEC");

    g(0, "SECTION");
    g(2, "ENTITIES");
    let lname = |l: Layer| match l {
        Layer::Line => "LINE",
        Layer::Hatch => "HATCH",
        Layer::Text => "TEXT",
        Layer::Fill => "FILL",
    };
    let ltname = |lt: LineType| match lt {
        LineType::Solid => "CONTINUOUS",
        LineType::Dashed => "DASHED",
        LineType::Chain => "CENTER",
    };
    for e in doc.ents.iter().filter(|e| o.has(e.layer)) {
        let head = |g: &mut dyn FnMut(i32, &str), kind: &str| {
            g(0, kind);
            g(8, lname(e.layer));
            if e.lt != LineType::Solid {
                g(6, ltname(e.lt));
            }
        };
        let poly = |g: &mut dyn FnMut(i32, &str), pts: &[[f32; 2]], closed: bool| {
            g(0, "POLYLINE");
            g(8, lname(e.layer));
            if e.lt != LineType::Solid {
                g(6, ltname(e.lt));
            }
            g(66, "1");
            g(70, if closed { "1" } else { "0" });
            for q in pts {
                g(0, "VERTEX");
                g(8, lname(e.layer));
                g(10, &f(x(q[0])));
                g(20, &f(y(q[1])));
            }
            g(0, "SEQEND");
            g(8, lname(e.layer));
        };
        match &e.shape {
            Shape::Line { a, b } => {
                head(&mut g, "LINE");
                g(10, &f(x(a[0])));
                g(20, &f(y(a[1])));
                g(11, &f(x(b[0])));
                g(21, &f(y(b[1])));
            }
            Shape::Poly { pts, closed } => poly(&mut g, pts, *closed),
            // R12 には楕円がないので、閉じた折れ線で出力する
            Shape::Ellipse { c, rx, ry, rot } => {
                let mut pts = crate::doc::ellipse_points(*c, *rx, *ry, *rot, 96);
                pts.pop();
                poly(&mut g, &pts, true)
            }
            Shape::Circle { c, r } => {
                head(&mut g, "CIRCLE");
                g(10, &f(x(c[0])));
                g(20, &f(y(c[1])));
                g(40, &f(*r as f64 * k));
            }
            Shape::Arc { c, r, a0, sweep } => {
                // 画像座標 (y 下向き) の角度 θ は DXF (y 上向き) では -θ。DXF の円弧は反時計回り。
                let (s0, s1) = if *sweep < 0.0 {
                    (-*a0, -(*a0 + *sweep))
                } else {
                    (-(*a0 + *sweep), -*a0)
                };
                head(&mut g, "ARC");
                g(10, &f(x(c[0])));
                g(20, &f(y(c[1])));
                g(40, &f(*r as f64 * k));
                g(50, &f((s0 as f64).to_degrees().rem_euclid(360.0)));
                g(51, &f((s1 as f64).to_degrees().rem_euclid(360.0)));
            }
            Shape::Text {
                bbox,
                s: txt,
                strokes,
            } => {
                if !txt.is_empty() {
                    g(0, "TEXT");
                    g(8, "TEXT");
                    g(10, &f(x(bbox[0])));
                    g(20, &f(y(bbox[3])));
                    g(40, &f(((bbox[3] - bbox[1]) as f64 * k).max(0.1)));
                    g(1, &txt.replace(['\r', '\n'], " "));
                    g(7, "STANDARD");
                } else if o.text_strokes {
                    for st in strokes {
                        if st.len() >= 2 {
                            poly(&mut g, st, false);
                        }
                    }
                }
            }
        }
    }
    g(0, "ENDSEC");
    g(0, "EOF");
    let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(&s);
    bytes.into_owned()
}

/// 円弧を折れ線にして返す (プレビュー描画用)。
pub fn arc_polyline(c: [f32; 2], r: f32, a0: f32, sweep: f32) -> Vec<[f32; 2]> {
    arc_points(c, r, a0, sweep, 2.0)
}
