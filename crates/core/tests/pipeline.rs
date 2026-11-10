//! 合成した線画でパイプライン全体を確認する。

use trace_core::doc::{Editor, Layer, LineType, Shape};
use trace_core::export::{to_dxf, to_svg};
use trace_core::pipeline::{prepare, vectorize, Params};
use trace_core::raster::Gray;
use trace_core::ExportOpts;

struct Canvas(Gray);

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Canvas(Gray::new(w, h, 235))
    }
    /// 太さ `t` の線分を描く。
    fn line(&mut self, a: (f64, f64), b: (f64, f64), t: f64) {
        let (w, h) = (self.0.w as i64, self.0.h as i64);
        let (x0, x1) = ((a.0.min(b.0) - t) as i64, (a.0.max(b.0) + t) as i64 + 1);
        let (y0, y1) = ((a.1.min(b.1) - t) as i64, (a.1.max(b.1) + t) as i64 + 1);
        for y in y0.max(0)..y1.min(h) {
            for x in x0.max(0)..x1.min(w) {
                let d = trace_core::geom::dist_seg([x as f64, y as f64], [a.0, a.1], [b.0, b.1]);
                if d <= t / 2.0 {
                    self.0.d[(y * w + x) as usize] = 40;
                }
            }
        }
    }
    fn dashed(&mut self, a: (f64, f64), b: (f64, f64), dash: f64, gap: f64, t: f64) {
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let (ux, uy) = ((b.0 - a.0) / len, (b.1 - a.1) / len);
        let mut s = 0.0;
        while s < len {
            let e = (s + dash).min(len);
            self.line(
                (a.0 + ux * s, a.1 + uy * s),
                (a.0 + ux * e, a.1 + uy * e),
                t,
            );
            s += dash + gap;
        }
    }
    fn circle(&mut self, c: (f64, f64), r: f64, t: f64) {
        let n = 180;
        for i in 0..n {
            let (a0, a1) = (
                i as f64 / n as f64 * std::f64::consts::TAU,
                (i + 1) as f64 / n as f64 * std::f64::consts::TAU,
            );
            self.line(
                (c.0 + r * a0.cos(), c.1 + r * a0.sin()),
                (c.0 + r * a1.cos(), c.1 + r * a1.sin()),
                t,
            );
        }
    }
}

fn lines(doc: &trace_core::Document, layer: Layer) -> Vec<([f32; 2], [f32; 2], LineType)> {
    doc.ents
        .iter()
        .filter(|e| e.layer == layer)
        .filter_map(|e| match e.shape {
            Shape::Line { a, b } => Some((a, b, e.lt)),
            _ => None,
        })
        .collect()
}

fn len(l: &([f32; 2], [f32; 2], LineType)) -> f32 {
    ((l.1[0] - l.0[0]).powi(2) + (l.1[1] - l.0[1]).powi(2)).sqrt()
}

fn trace(c: &Canvas, p: &Params) -> trace_core::Document {
    let prep = prepare(&c.0, p);
    vectorize(&prep.bin, p).0
}

fn no_rectify() -> Params {
    Params {
        rectify: trace_core::rectify::RectifyMode::None,
        ..Default::default()
    }
}

#[test]
fn rectangle_keeps_sharp_corners() {
    let mut c = Canvas::new(600, 400);
    for (a, b) in [
        ((100., 80.), (500., 80.)),
        ((500., 80.), (500., 320.)),
        ((500., 320.), (100., 320.)),
        ((100., 320.), (100., 80.)),
    ] {
        c.line(a, b, 3.0);
    }
    let doc = trace(&c, &no_rectify());
    let ls = lines(&doc, Layer::Line);
    assert_eq!(ls.len(), 4, "長方形は 4 本の線になる: {ls:?}");
    // すべての端点が角 (±2px) に乗っている = 角が丸められていない
    for l in &ls {
        for p in [l.0, l.1] {
            let ok = [(100., 80.), (500., 80.), (500., 320.), (100., 320.)]
                .iter()
                .any(|q: &(f32, f32)| (p[0] - q.0).abs() < 2.0 && (p[1] - q.1).abs() < 2.0);
            assert!(ok, "端点が角にない: {p:?}");
        }
        assert!(
            l.0[0] == l.1[0] || l.0[1] == l.1[1],
            "水平・垂直にそろっている"
        );
    }
}

#[test]
fn crossing_lines_stay_whole() {
    let mut c = Canvas::new(500, 500);
    c.line((50., 250.), (450., 250.), 3.0);
    c.line((250., 50.), (250., 450.), 3.0);
    let doc = trace(&c, &no_rectify());
    let ls = lines(&doc, Layer::Line);
    assert_eq!(ls.len(), 2, "交差しても 2 本のまま: {ls:?}");
    assert!(ls.iter().all(|l| len(l) > 390.0));
}

#[test]
fn dashed_line_becomes_one_entity() {
    let mut c = Canvas::new(700, 300);
    c.dashed((60., 100.), (640., 100.), 24.0, 10.0, 3.0);
    c.line((60., 200.), (640., 200.), 3.0);
    let doc = trace(&c, &no_rectify());
    let ls = lines(&doc, Layer::Line);
    assert_eq!(ls.len(), 2, "{ls:?}");
    let dashed: Vec<_> = ls.iter().filter(|l| l.2 == LineType::Dashed).collect();
    assert_eq!(dashed.len(), 1);
    assert!(
        len(dashed[0]) > 560.0,
        "破線全体が 1 本: {}",
        len(dashed[0])
    );
    let dxf = String::from_utf8_lossy(&to_dxf(&doc, &ExportOpts::default())).into_owned();
    assert!(dxf.contains("DASHED"));
    assert!(to_svg(&doc, &ExportOpts::default()).contains("stroke-dasharray"));
}

#[test]
fn hatching_is_separated() {
    let mut c = Canvas::new(600, 400);
    for (a, b) in [
        ((100., 100.), (500., 100.)),
        ((500., 100.), (500., 300.)),
        ((500., 300.), (100., 300.)),
        ((100., 300.), (100., 100.)),
    ] {
        c.line(a, b, 3.0);
    }
    // 45° のハッチング
    let mut k = -200.0;
    while k < 400.0 {
        let (mut ax, mut ay, mut bx, mut by) = (100.0 + k, 300.0, 100.0 + k + 200.0, 100.0);
        if ax < 100.0 {
            ay -= 100.0 - ax;
            ax = 100.0;
        }
        if bx > 500.0 {
            by += bx - 500.0;
            bx = 500.0;
        }
        if bx - ax > 12.0 {
            c.line((ax, ay), (bx, by), 2.0);
        }
        k += 16.0;
    }
    let doc = trace(&c, &no_rectify());
    let outline = lines(&doc, Layer::Line);
    let hatch = lines(&doc, Layer::Hatch);
    assert_eq!(outline.len(), 4, "輪郭だけが線レイヤに残る: {outline:?}");
    assert!(
        hatch.len() >= 25,
        "ハッチングは別レイヤ: {} 本",
        hatch.len()
    );
    // 既定の出力にはハッチングが含まれない
    let svg = to_svg(&doc, &ExportOpts::default());
    assert_eq!(svg.matches("<line").count(), 4);
    // 無効にすれば普通の線として残る
    let doc2 = trace(
        &c,
        &Params {
            hatch: false,
            ..no_rectify()
        },
    );
    assert!(lines(&doc2, Layer::Line).len() >= 29);
}

#[test]
fn circle_is_detected() {
    let mut c = Canvas::new(400, 400);
    c.circle((200., 200.), 90.0, 3.0);
    let doc = trace(&c, &no_rectify());
    let circles: Vec<_> = doc
        .ents
        .iter()
        .filter_map(|e| {
            if let Shape::Circle { c, r } = e.shape {
                Some((c, r))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(circles.len(), 1, "{:?}", doc.ents);
    assert!((circles[0].1 - 90.0).abs() < 2.0 && (circles[0].0[0] - 200.0).abs() < 2.0);
}

#[test]
fn skew_is_corrected() {
    let mut c = Canvas::new(900, 700);
    let th = 3.0f64.to_radians();
    let rot = |x: f64, y: f64| -> (f64, f64) {
        let (dx, dy) = (x - 450.0, y - 350.0);
        (
            450.0 + dx * th.cos() - dy * th.sin(),
            350.0 + dx * th.sin() + dy * th.cos(),
        )
    };
    for i in 0..5 {
        let y = 120.0 + i as f64 * 110.0;
        c.line(rot(120.0, y), rot(780.0, y), 3.0);
        let x = 120.0 + i as f64 * 165.0;
        c.line(rot(x, 120.0), rot(x, 560.0), 3.0);
    }
    let p = Params::default();
    let prep = prepare(&c.0, &p);
    let (doc, _) = vectorize(&prep.bin, &p);
    let ls = lines(&doc, Layer::Line);
    assert_eq!(ls.len(), 10, "{ls:?}");
    assert!(
        ls.iter().all(|l| l.0[0] == l.1[0] || l.0[1] == l.1[1]),
        "補正後はすべて水平・垂直"
    );
}

#[test]
fn editor_join_and_undo() {
    let mut doc = trace_core::Document {
        w: 100.0,
        h: 100.0,
        sw: 2.0,
        ..Default::default()
    };
    for i in 0..4 {
        let x = 10.0 + i as f32 * 20.0;
        doc.push(
            Shape::Line {
                a: [x, 50.0],
                b: [x + 12.0, 50.2],
            },
            Layer::Line,
            LineType::Solid,
            0,
        );
    }
    doc.push(
        Shape::Line {
            a: [50.0, 10.0],
            b: [50.0, 30.0],
        },
        Layer::Line,
        LineType::Solid,
        0,
    );
    let mut ed = Editor::new(doc);
    // 範囲選択 (交差) で 4 片を選んで接続 → 破線 1 本
    ed.select_rect([0.0, 40.0, 100.0, 60.0], 1, false, true);
    assert_eq!(ed.sel.len(), 4);
    assert_eq!(ed.join_selected(), 1);
    assert_eq!(ed.doc.ents.len(), 2);
    let joined = ed.doc.ents.last().unwrap();
    assert_eq!(joined.lt, LineType::Dashed);
    assert!(ed.undo());
    assert_eq!(ed.doc.ents.len(), 5);
    assert!(ed.redo());
    // 交わる 2 本は角になる
    ed.select_all(1);
    assert_eq!(ed.sel.len(), 2);
    assert_eq!(ed.join_selected(), 1);
    let v = ed.doc.ents.iter().find_map(|e| match e.shape {
        Shape::Line { a, b } if a[0] == b[0] => Some((a, b)),
        _ => None,
    });
    let (a, b) = v.expect("垂直線");
    assert!(
        (a[1].max(b[1]) - 50.1).abs() < 0.5,
        "垂直線が水平線まで延びる: {a:?} {b:?}"
    );
    // ハッチング扱いにして出力から外す
    ed.set_layer(Layer::Hatch);
    assert!(ed.doc.ents.iter().all(|e| e.layer == Layer::Hatch));
    assert_eq!(
        to_svg(&ed.doc, &ExportOpts::default())
            .matches("<line")
            .count(),
        0
    );
}
