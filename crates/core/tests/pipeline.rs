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

fn line_doc(lines: &[([f32; 2], [f32; 2])]) -> Editor {
    let mut doc = trace_core::Document {
        w: 1000.0,
        h: 1000.0,
        sw: 2.0,
        ..Default::default()
    };
    for (a, b) in lines {
        doc.push(
            Shape::Line { a: *a, b: *b },
            Layer::Line,
            LineType::Solid,
            0,
        );
    }
    Editor::new(doc)
}

fn line_of(ed: &Editor, i: usize) -> ([f32; 2], [f32; 2]) {
    match ed.doc.ents[i].shape {
        Shape::Line { a, b } => (a, b),
        _ => panic!("line"),
    }
}

#[test]
fn whiskers_are_trimmed() {
    // L 字の角で両方の線が 6px ずつはみ出している + 目盛り (短い交差線) は残す
    let mut ed = line_doc(&[
        ([100.0, 200.0], [506.0, 200.0]),
        ([500.0, 194.0], [500.0, 600.0]),
        ([300.0, 190.0], [300.0, 210.0]),
    ]);
    assert_eq!(ed.trim_whiskers(10.0, false), 2);
    let (a, b) = line_of(&ed, 0);
    assert!(
        (a[0] - 100.0).abs() < 0.01 && (b[0] - 500.0).abs() < 0.01,
        "{a:?} {b:?}"
    );
    let (a, b) = line_of(&ed, 1);
    assert!(
        (a[1] - 200.0).abs() < 0.01 && (b[1] - 600.0).abs() < 0.01,
        "{a:?} {b:?}"
    );
    let (a, b) = line_of(&ed, 2);
    assert_eq!((a[1], b[1]), (190.0, 210.0), "目盛りはそのまま");
    // 長いはみ出しは対象外
    let mut ed = line_doc(&[
        ([100.0, 200.0], [560.0, 200.0]),
        ([500.0, 100.0], [500.0, 600.0]),
    ]);
    assert_eq!(ed.trim_whiskers(10.0, false), 0);
    // なぞって消す: はみ出した先をなぞると交点まで切り詰められ、Undo は 1 回で戻る
    ed.begin_group();
    assert!(ed.trim_at([540.0, 201.0], 4.0, 1));
    assert!(ed.trim_at([500.0, 150.0], 4.0, 1));
    ed.end_group();
    let (_, b) = line_of(&ed, 0);
    assert!((b[0] - 500.0).abs() < 0.01);
    let (a, _) = line_of(&ed, 1);
    assert!((a[1] - 200.0).abs() < 0.01);
    assert!(ed.undo());
    assert_eq!(line_of(&ed, 0).1[0], 560.0);
    assert_eq!(line_of(&ed, 1).0[1], 100.0);
}

#[test]
fn width_is_equalised() {
    // 少し傾いて間隔 18〜22px の 2 本 → 平行で間隔 30
    let mut ed = line_doc(&[
        ([100.0, 100.0], [500.0, 102.0]),
        ([500.0, 124.0], [100.0, 118.0]),
    ]);
    ed.select_all(1);
    ed.equalize_width(30.0).unwrap();
    let (a0, b0) = line_of(&ed, 0);
    let (a1, b1) = line_of(&ed, 1);
    let d = trace_core::geom::unit([(b0[0] - a0[0]) as f64, (b0[1] - a0[1]) as f64]);
    for q in [a1, b1] {
        let off = trace_core::geom::line_offset(
            [q[0] as f64, q[1] as f64],
            [a0[0] as f64, a0[1] as f64],
            d,
        )
        .abs();
        assert!((off - 30.0).abs() < 0.01, "間隔 {off}");
    }
    assert!(a1[0] > 400.0 && b1[0] < 200.0, "端点の向きは元のまま");
    // 折れ線 (L 字の壁) も同じ幅にそろう
    let mut doc = trace_core::Document {
        w: 1000.0,
        h: 1000.0,
        sw: 2.0,
        ..Default::default()
    };
    doc.push(
        Shape::Poly {
            pts: vec![[100.0, 100.0], [400.0, 100.0], [400.0, 400.0]],
            closed: false,
        },
        Layer::Line,
        LineType::Solid,
        0,
    );
    doc.push(
        Shape::Poly {
            pts: vec![[100.0, 122.0], [381.0, 121.0], [380.0, 400.0]],
            closed: false,
        },
        Layer::Line,
        LineType::Solid,
        0,
    );
    let mut ed = Editor::new(doc);
    ed.select_all(1);
    ed.equalize_width(20.0).unwrap();
    let (Shape::Poly { pts: p0, .. }, Shape::Poly { pts: p1, .. }) =
        (&ed.doc.ents[0].shape, &ed.doc.ents[1].shape)
    else {
        panic!()
    };
    // 角の頂点どうしは対角方向に 20*sqrt(2) 離れる
    let corner = ((p0[1][0] - p1[1][0]).powi(2) + (p0[1][1] - p1[1][1]).powi(2)).sqrt();
    assert!((corner - 20.0 * 2f32.sqrt()).abs() < 0.3, "{corner}");
}

#[test]
fn dimensions_calibrate_scale_and_skew() {
    // 実寸 1000x500mm の長方形を、横 0.25・縦 0.26 倍 + わずかなせん断で歪ませた図
    let warp = |x: f32, y: f32| -> [f32; 2] { [0.25 * x + 0.004 * y + 50.0, 0.26 * y + 80.0] };
    let c = [
        warp(0.0, 0.0),
        warp(1000.0, 0.0),
        warp(1000.0, 500.0),
        warp(0.0, 500.0),
    ];
    let mut ed = line_doc(&[(c[0], c[1]), (c[1], c[2]), (c[2], c[3]), (c[3], c[0])]);
    let f = |q: [f32; 2]| [q[0] as f64, q[1] as f64];
    ed.add_dim(f(c[0]), f(c[1]), 1000.0);
    ed.add_dim(f(c[1]), f(c[2]), 500.0);
    ed.add_dim(
        f(c[0]),
        f(c[2]),
        (1000.0f64.powi(2) + 500.0f64.powi(2)).sqrt(),
    );
    let msg = ed.calibrate().unwrap();
    assert!(msg.contains("せん断"), "{msg}");
    let unit = ed.doc.calib.unit as f64;
    let len = |i: usize| {
        let (a, b) = line_of(&ed, i);
        (((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)) as f64).sqrt() * unit
    };
    for (i, want) in [(0, 1000.0), (1, 500.0), (2, 1000.0), (3, 500.0)] {
        assert!((len(i) - want).abs() < 1.0, "辺 {i}: {} mm", len(i));
    }
    // 直角も戻っている
    let (a, b) = line_of(&ed, 0);
    let (c0, d0) = line_of(&ed, 1);
    let dot = (b[0] - a[0]) * (d0[0] - c0[0]) + (b[1] - a[1]) * (d0[1] - c0[1]);
    assert!((dot as f64).abs() / (len(0) / unit * len(1) / unit) < 2e-3);
    // 寸法 1 本だけなら縮尺のみ
    let mut ed = line_doc(&[([0.0, 0.0], [200.0, 0.0])]);
    ed.add_dim([0.0, 0.0], [200.0, 0.0], 5000.0);
    ed.calibrate().unwrap();
    assert!((ed.doc.calib.unit - 25.0).abs() < 1e-3);
    assert_eq!(line_of(&ed, 0).1[0], 200.0);
}

#[test]
fn trim_stroke_only_cuts_crossed_lines_once() {
    // 上辺を 8px 突き抜けた縦線。ヒゲを横切るようになぞる。
    let mut ed = line_doc(&[
        ([100.0, 200.0], [600.0, 200.0]),
        ([300.0, 192.0], [300.0, 500.0]),
        ([100.0, 350.0], [600.0, 350.0]),
    ]);
    ed.begin_group();
    assert!(ed.trim_stroke([294.0, 196.0], [300.5, 196.0], 1));
    // なぞり続けても同じ線の次の区間や、近くを通っただけの上辺には触れない
    assert!(!ed.trim_stroke([300.5, 196.0], [306.0, 197.0], 1));
    assert!(!ed.trim_stroke([306.0, 197.0], [299.0, 199.0], 1));
    ed.end_group();
    assert_eq!(ed.doc.ents.len(), 3);
    assert_eq!(line_of(&ed, 0), ([100.0, 200.0], [600.0, 200.0]));
    let (a, b) = line_of(&ed, 1);
    assert!((a[1] - 200.0).abs() < 0.01 && b[1] == 500.0, "{a:?} {b:?}");
}

#[test]
fn grid_paper_lines_are_removed_together() {
    // 方眼紙 (25px 間隔の細い線) の上に、太い線で四角を描いた図。方眼は紙の伸びで間隔がわずかに違う。
    let mut c = Canvas::new(700, 500);
    let (px, py) = (25.3, 24.8);
    let mut x = 30.0;
    while x < 680.0 {
        c.line((x, 10.0), (x, 490.0), 1.6);
        x += px;
    }
    let mut y = 20.0;
    while y < 490.0 {
        c.line((10.0, y), (690.0, y), 1.6);
        y += py;
    }
    // 図: 方眼から外れた位置の太い四角 + 方眼の線の真上に描いた太い線
    for (a, b) in [
        ((143.0, 108.0), (520.0, 108.0)),
        ((520.0, 108.0), (520.0, 381.0)),
        ((520.0, 381.0), (143.0, 381.0)),
        ((143.0, 381.0), (143.0, 108.0)),
    ] {
        c.line(a, b, 5.0);
    }
    let p = Params {
        hatch: false,
        text: false,
        fill: false,
        ..no_rectify()
    };
    let doc = trace(&c, &p);
    // 線が細いので 2 倍に拡大して処理される。期待値の座標もその倍率に合わせる。
    let k = (doc.w / 700.0) as f64;
    let total = lines(&doc, Layer::Line).len();
    assert!(total > 40, "方眼が線として拾われている: {total}");
    let mut ed = Editor::new(doc);
    // 細い方眼の線を、縦 3 本・横 3 本だけ選ぶ (図と交差して分断されていない端のほう)
    let pick = |ed: &Editor, horizontal: bool, n: usize| -> Vec<u32> {
        let mut v: Vec<(f32, u32)> = ed
            .doc
            .ents
            .iter()
            .filter_map(|e| match e.shape {
                Shape::Line { a, b }
                    if e.w < 3.0
                        && ((a[1] == b[1]) == horizontal)
                        && (a[0] == b[0]) != horizontal =>
                {
                    let len = (b[0] - a[0]).abs() + (b[1] - a[1]).abs();
                    (len > 400.0 * k as f32).then_some((if horizontal { a[1] } else { a[0] }, e.id))
                }
                _ => None,
            })
            .collect();
        v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        v.iter().take(n).map(|x| x.1).collect()
    };
    for id in pick(&ed, true, 3).into_iter().chain(pick(&ed, false, 3)) {
        ed.sel.insert(id);
    }
    assert_eq!(ed.sel.len(), 6);
    let removed = ed.remove_grid(1).unwrap();
    assert!(
        removed + 6 >= total,
        "ほぼすべての方眼線が消える: {removed}/{total}"
    );
    // 太い四角の 4 辺は残っている
    let rest = lines(&ed.doc, Layer::Line);
    for (x0, y0, x1, y1) in [
        (143., 108., 520., 108.),
        (520., 108., 520., 381.),
        (143., 381., 520., 381.),
        (143., 108., 143., 381.),
    ] {
        let covered: f32 = rest
            .iter()
            .filter(|l| {
                let on = |p: [f32; 2]| {
                    trace_core::geom::dist_seg(
                        [p[0] as f64, p[1] as f64],
                        [x0 * k, y0 * k],
                        [x1 * k, y1 * k],
                    ) < 3.0
                };
                on(l.0) && on(l.1)
            })
            .map(len)
            .sum();
        let want = (((x1 - x0).abs() + (y1 - y0).abs()) * k) as f32;
        assert!(covered > 0.9 * want, "図の線が残っている: {covered}/{want}");
    }
    assert!(rest.len() <= 12, "残りは図の線だけ: {}", rest.len());
    assert!(ed.undo());
    assert_eq!(lines(&ed.doc, Layer::Line).len(), total);
}

#[test]
fn join_pen_connects_touched_fragments() {
    // かすれて 3 つに切れた水平線 + 近くを横切る縦線 + 少し離れた平行線
    let mut ed = line_doc(&[
        ([100.0, 200.0], [220.0, 201.0]),
        ([240.0, 200.5], [300.0, 200.0]),
        ([320.0, 200.0], [480.0, 199.5]),
        ([260.0, 100.0], [260.0, 300.0]),
        ([100.0, 240.0], [480.0, 240.0]),
    ]);
    // 半径 12 のペンで線に沿ってなぞる。横切っている縦線にも触れるが、選ばれるのは沿っている 3 片だけ
    let path: Vec<[f64; 2]> = (0..=12)
        .map(|i| [210.0 + i as f64 * 10.0, 205.0 - i as f64 * 0.7])
        .collect();
    ed.select_along_stroke(&path, 12.0, 1);
    assert_eq!(
        ed.sel.len(),
        3,
        "沿っている 3 片だけ (横切る縦線・離れた平行線は選ばれない)"
    );
    // 縦になぞれば、今度は縦線だけ
    let vpath: Vec<[f64; 2]> = (0..=10).map(|i| [262.0, 150.0 + i as f64 * 10.0]).collect();
    ed.select_along_stroke(&vpath, 12.0, 1);
    assert_eq!(ed.sel.len(), 1);
    // 短い線 (中央の 1 片) をぐりぐり囲めば、向きに関係なく完全に含まれる線として選ばれる
    let blob: Vec<[f64; 2]> = vec![
        [245.0, 195.0],
        [270.0, 206.0],
        [295.0, 195.0],
        [270.0, 200.0],
        [245.0, 205.0],
        [296.0, 204.0],
    ];
    ed.select_along_stroke(&blob, 12.0, 1);
    assert!(ed.sel.len() == 1, "中央の短い 1 片だけ: {:?}", ed.sel);
    ed.select_along_stroke(&path, 12.0, 1);
    assert_eq!(ed.join_selected_as(true), 1);
    assert_eq!(ed.doc.ents.len(), 3);
    let joined = ed.doc.ents.last().unwrap();
    assert_eq!(joined.lt, LineType::Solid, "つなぎペンは実線としてつなぐ");
    let Shape::Line { a, b } = joined.shape else {
        panic!()
    };
    assert!((a[0].min(b[0]) - 100.0).abs() < 0.5 && (a[0].max(b[0]) - 480.0).abs() < 0.5);
    // 縦線はそのまま
    assert_eq!(line_of(&ed, 0), ([260.0, 100.0], [260.0, 300.0]));
    assert!(ed.undo());
    assert_eq!(ed.doc.ents.len(), 5);
}

#[test]
fn thick_bars_become_lines() {
    // ハッチングがつぶれて塗りになった壁を想定: 太さ 24px の L 字の帯 (輪郭は少しガタガタ)
    let mut c = Canvas::new(700, 500);
    c.line((100.0, 100.0), (600.0, 100.0), 24.0);
    c.line((600.0, 100.0), (600.0, 400.0), 24.0);
    for i in 0..40 {
        // 輪郭の凹凸
        let x = 110.0 + i as f64 * 12.0;
        c.line((x, 112.0), (x + 3.0, 114.0), 3.0);
    }
    // 普通の細い線 (図面の大半は細い線なので、代表線幅は細くなる)
    for i in 0..8 {
        let y = 200.0 + i as f64 * 30.0;
        c.line((60.0, y), (500.0, y), 3.0);
    }
    let p = Params {
        text: false,
        hatch: false,
        ..no_rectify()
    };
    let doc = trace(&c, &p);
    assert!(
        doc.ents.iter().any(|e| e.layer == Layer::Fill),
        "太い帯は塗りとして拾われる"
    );
    let plain = lines(&doc, Layer::Line).len();

    // 2 本線: 帯の両側が水平・垂直の線になる
    let mut ed = Editor::new(doc.clone());
    let (nf, nl) = ed.fills_to_lines(false);
    assert!(nf >= 1 && nl >= 4, "{nf} {nl}");
    assert!(ed.doc.ents.iter().all(|e| e.layer != Layer::Fill));
    let ls = lines(&ed.doc, Layer::Line);
    assert!(
        ls.iter().all(|l| l.0[0] == l.1[0] || l.0[1] == l.1[1]),
        "水平・垂直にそろう: {ls:?}"
    );
    let near = |v: f32, want: f32| (v - want).abs() < 4.0;
    assert!(
        ls.iter()
            .any(|l| l.0[1] == l.1[1] && near(l.0[1], 88.0) && len(l) > 450.0),
        "上辺: {ls:?}"
    );
    assert!(
        ls.iter()
            .any(|l| l.0[1] == l.1[1] && near(l.0[1], 112.0) && len(l) > 400.0),
        "下辺 (凹凸はならされる): {ls:?}"
    );
    assert!(
        ls.iter()
            .any(|l| l.0[0] == l.1[0] && near(l.0[0], 612.0) && len(l) > 250.0),
        "右辺: {ls:?}"
    );
    assert!(ed.undo());
    assert!(ed.doc.ents.iter().any(|e| e.layer == Layer::Fill));

    // 中心線: 帯 1 本につき線 1 本
    let mut ed = Editor::new(doc);
    let (_, nl) = ed.fills_to_lines(true);
    let ls = lines(&ed.doc, Layer::Line);
    assert_eq!(ls.len(), plain + nl);
    assert!(
        ls.iter()
            .any(|l| l.0[1] == l.1[1] && near(l.0[1], 100.0) && len(l) > 450.0),
        "水平の中心線: {ls:?}"
    );
    assert!(
        ls.iter()
            .any(|l| l.0[0] == l.1[0] && near(l.0[0], 600.0) && len(l) > 250.0),
        "垂直の中心線: {ls:?}"
    );
    assert!(nl <= 3, "中心線は帯ごとに 1 本: {nl}");
}

#[test]
fn clearly_continuing_lines_are_bridged() {
    // 壁 (2 本線) と文字に遮られて途切れた長い線。延長すれば明らかに 1 本。
    let mut c = Canvas::new(900, 500);
    c.line((40.0, 250.0), (300.0, 250.0), 3.0);
    c.line((340.0, 250.0), (560.0, 250.0), 3.0); // すき間 40px (間に壁)
    c.line((640.0, 250.0), (860.0, 250.0), 3.0); // すき間 80px (何もない)
    c.line((310.0, 60.0), (310.0, 440.0), 3.0);
    c.line((330.0, 60.0), (330.0, 440.0), 3.0);
    // 開口部: 同じ直線上の 2 本だが、両側とも直交する線 (建具枠) で止めてある → つながない
    c.line((40.0, 400.0), (250.0, 400.0), 3.0);
    c.line((250.0, 380.0), (250.0, 400.0), 3.0);
    c.line((290.0, 400.0), (290.0, 380.0), 3.0);
    // (右側は壁 x=310 に当たるので短い線を別に置く)
    c.line((290.0, 400.0), (300.0, 400.0), 3.0);
    let doc = trace(
        &c,
        &Params {
            text: false,
            ..no_rectify()
        },
    );
    let ls = lines(&doc, Layer::Line);
    let long: Vec<_> = ls
        .iter()
        .filter(|l| l.0[1] == l.1[1] && (l.0[1] - 250.0).abs() < 2.0)
        .collect();
    assert_eq!(long.len(), 1, "途切れた線は 1 本につながる: {long:?}");
    assert!(len(long[0]) > 800.0);
    let door: Vec<_> = ls
        .iter()
        .filter(|l| l.0[1] == l.1[1] && (l.0[1] - 400.0).abs() < 2.0)
        .collect();
    assert!(
        door.iter().all(|l| len(l) < 230.0),
        "角で止めた線は開口部をまたいでつながない: {door:?}"
    );
    // 無効にすればつながらない
    let doc = trace(
        &c,
        &Params {
            text: false,
            bridge_gap: 0.0,
            ..no_rectify()
        },
    );
    let n = lines(&doc, Layer::Line)
        .iter()
        .filter(|l| l.0[1] == l.1[1] && (l.0[1] - 250.0).abs() < 2.0)
        .count();
    assert_eq!(n, 3);
}

#[test]
fn solid_squares_and_discs_become_outlines() {
    // 塗りつぶした柱 (60px 角) に壁の 2 本線がつながり、離れた所に塗りつぶした円がある
    let mut c = Canvas::new(700, 500);
    for i in 0..60 {
        let y = 100.5 + i as f64;
        c.line((100.5, y), (159.5, y), 1.0);
    }
    c.line((160.0, 120.0), (600.0, 120.0), 3.0);
    c.line((160.0, 140.0), (600.0, 140.0), 3.0);
    c.line((600.0, 120.0), (600.0, 140.0), 3.0);
    for i in 0..8 {
        let y = 220.0 + i as f64 * 30.0;
        c.line((60.0, y), (300.0, y), 3.0);
    }
    for r in 0..30 {
        c.circle((500.0, 320.0), r as f64, 2.0);
    }
    let p = Params {
        text: false,
        hatch: false,
        ..no_rectify()
    };
    let doc = trace(&c, &p);
    assert!(
        doc.ents.iter().all(|e| e.layer != Layer::Fill),
        "矩形・円の塗りは残らない"
    );
    let ls = lines(&doc, Layer::Line);
    let near = |v: f32, want: f32| (v - want).abs() < 1.6;
    // 柱の 4 辺 (水平・垂直ぴったり)
    for (x0, y0, x1, y1) in [
        (100.0, 100.0, 160.0, 100.0),
        (160.0, 100.0, 160.0, 160.0),
        (100.0, 160.0, 160.0, 160.0),
        (100.0, 100.0, 100.0, 160.0),
    ] {
        assert!(
            ls.iter().any(|l| {
                let (a, b) = if (l.0[0], l.0[1]) <= (l.1[0], l.1[1]) {
                    (l.0, l.1)
                } else {
                    (l.1, l.0)
                };
                (a[0] == b[0] || a[1] == b[1])
                    && near(a[0], x0)
                    && near(a[1], y0)
                    && near(b[0], x1)
                    && near(b[1], y1)
            }),
            "辺 ({x0},{y0})-({x1},{y1}): {ls:?}"
        );
    }
    // 壁の線は柱の辺まで届く
    for y in [120.0, 140.0] {
        assert!(
            ls.iter().any(|l| near(l.0[1], y)
                && near(l.1[1], y)
                && near(l.0[0].min(l.1[0]), 160.0)
                && len(l) > 400.0),
            "壁 y={y}: {ls:?}"
        );
    }
    // 円
    assert!(
        doc.ents.iter().any(|e| matches!(&e.shape,
            Shape::Circle { c, r } if near(c[0], 500.0) && near(c[1], 320.0) && (*r - 30.0).abs() < 2.5)),
        "塗りつぶした円は円になる"
    );

    // 無効にすれば従来どおり塗りのまま
    let doc = trace(
        &c,
        &Params {
            fill_shapes: false,
            ..p
        },
    );
    assert_eq!(
        doc.ents.iter().filter(|e| e.layer == Layer::Fill).count(),
        2
    );
}
