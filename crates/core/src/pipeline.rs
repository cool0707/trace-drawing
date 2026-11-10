//! 画像 → ベクタ図面の全体パイプライン。

use crate::binarize;
use crate::doc::{Document, Layer, LineType, Shape};
use crate::fit::{fit_chain, FitOpts, Prim};
use crate::geom::*;
use crate::morph;
use crate::post::{self, Seg, SmallKind};
use crate::raster::{self, Gray};
use crate::rectify::{self, RectifyMode};
use crate::skeleton;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 入力画像の種類。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// 紙の白さから自動判定
    Auto,
    /// 階調のある青焼きスキャン・写真 (背景ムラの除去とかすれ補完を行う)
    Blueprint,
    /// 2 値化済みのスキャン (単純なしきい値のみ)
    Binary,
}

/// ベタ塗りとして拾った太い帯の扱い。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FillAs {
    /// 塗り (輪郭のポリゴン) のまま
    Keep,
    /// 両側の 2 本線にする
    Outline,
    /// 中心線 1 本にする
    Center,
}

/// トレースのパラメータ。長さ系の値は「代表線幅の何倍か」で指定するので解像度に依存しない。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    /// 作業解像度 (長辺の最大画素数)。0 で縮小なし。
    pub max_side: usize,
    /// 読み込み時に時計回りに 90°×n 回転
    pub rot90: u8,
    /// 白黒反転: -1=自動, 0=しない, 1=する
    pub invert: i8,
    /// 入力画像の種類
    pub source: Source,
    /// 2 値スキャンのしきい値 (0 = 自動)。これより暗い画素を線とみなす
    pub threshold: u8,
    /// 線が極端に細い (約 1 画素の) 画像を 2 倍に拡大してから処理する
    pub upscale_thin: bool,
    /// 線の拾いやすさ 0〜1 (大きいほどかすれた線も拾うがゴミも増える)
    pub sensitivity: f64,
    /// これより小さい孤立したゴミを消す (線幅² の何倍の面積か)
    pub despeckle: f64,
    /// 傾き・歪み補正の方法
    pub rectify: RectifyMode,
    /// 手動補正: 元画像上の 4 隅 (指定時は自動推定より優先)
    pub corners: Option<[[f64; 2]; 4]>,
    /// 折れ線近似の許容誤差 (線幅の何倍か)
    pub simplify: f64,
    /// この角度以内の線は水平・垂直にそろえる [度] (0 で無効)
    pub snap_hv_deg: f64,
    /// これより短い線は捨てる (線幅の何倍か)
    pub min_len: f64,
    /// 交点からこの長さ以下だけはみ出した線の先 (ヒゲ) を自動で切り詰める (線幅の何倍か, 0 で無効)
    pub trim_overshoot: f64,
    /// 円・円弧を検出する
    pub arcs: bool,
    /// 破線・一点鎖線を 1 本の線にまとめる
    pub dashed: bool,
    /// 破線とみなす最大のすき間 (線幅の何倍か)
    pub dash_max_gap: f64,
    /// ハッチングを検出して別レイヤにする
    pub hatch: bool,
    /// ハッチングとみなす最大の線間隔 (線幅の何倍か)
    pub hatch_max_spacing: f64,
    /// ハッチングとみなす最少本数
    pub hatch_min_lines: usize,
    /// 水平・垂直の線の束もハッチングとみなす
    pub hatch_axis: bool,
    /// 文字らしい小さなかたまりを別レイヤにする
    pub text: bool,
    /// 文字とみなす最大サイズ (線幅の何倍か)
    pub text_max_size: f64,
    /// ベタ塗り領域を輪郭として取り出す
    pub fill: bool,
    /// ベタ塗りとみなす最小の太さ (線幅の何倍か)
    pub fill_min_width: f64,
    /// 塗りになった太い帯を線に変換するか
    pub fill_as: FillAs,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            max_side: 5000,
            rot90: 0,
            invert: -1,
            source: Source::Auto,
            threshold: 0,
            upscale_thin: true,
            sensitivity: 0.5,
            despeckle: 2.0,
            rectify: RectifyMode::Perspective,
            corners: None,
            simplify: 1.0,
            snap_hv_deg: 1.5,
            min_len: 3.0,
            trim_overshoot: 0.0,
            arcs: true,
            dashed: true,
            dash_max_gap: 14.0,
            hatch: true,
            hatch_max_spacing: 14.0,
            hatch_min_lines: 4,
            hatch_axis: false,
            text: true,
            text_max_size: 30.0,
            fill: true,
            fill_min_width: 5.0,
            fill_as: FillAs::Keep,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub width: usize,
    pub height: usize,
    pub stroke_width: f64,
    pub inverted: bool,
    /// 2 値スキャンとして処理したか
    pub binary_source: bool,
    /// 補正で直した傾き [度]
    pub skew_deg: f64,
    pub lines: usize,
    pub dashed: usize,
    pub hatch_lines: usize,
    pub hatch_groups: u32,
    pub arcs: usize,
    pub texts: usize,
    pub fills: usize,
}

/// 補正済みの画像一式。
pub struct Prepared {
    /// 背景を正規化した補正済みグレースケール (編集画面の下絵)
    pub norm: Gray,
    /// 二値画像 (1 = インク)
    pub bin: Gray,
    /// 元画像 (作業解像度) → 補正後 の射影変換
    pub h: M3,
    pub inverted: bool,
    /// 2 値スキャンとして処理したか
    pub clean: bool,
    /// 読み込んだ画像 → 作業画像 の拡大率 (縮小なら 1 未満、低解像度の拡大なら 2)
    pub scale: f64,
}

fn v2(p: P) -> [f32; 2] {
    [p[0] as f32, p[1] as f32]
}

fn binarize_clean(norm: &Gray, p: &Params, clean: bool) -> Gray {
    let mut bin = if clean {
        binarize::threshold_global(norm, p.threshold, p.sensitivity)
    } else {
        binarize::threshold(norm, p.sensitivity)
    };
    let dt = morph::chamfer(&bin, 1, true);
    let sw = morph::stroke_width(&bin, &dt);
    let min_area = (p.despeckle * sw * sw).max(4.0) as u32;
    let max_hole = (0.6 * sw * sw).max(2.0) as u32;
    morph::clean(&mut bin, min_area, max_hole);
    bin
}

/// 二値画像から直線の候補だけを粗く取り出す (歪み推定用)。
fn rough_segments(bin: &Gray) -> Vec<(P, P)> {
    let dt = morph::chamfer(bin, 1, true);
    let sw = morph::stroke_width(bin, &dt);
    let (labels, _) = morph::components(bin, 1, true);
    let mut sk = skeleton::thin(bin);
    let chains = skeleton::chains(&mut sk, &dt, &labels);
    let o = FitOpts {
        eps: sw.max(1.5),
        sw,
        arcs: false,
        max_r: 0.0,
    };
    let mut out = Vec::new();
    for c in &chains {
        for pr in fit_chain(&c.pts, c.closed, &o) {
            if let Prim::Seg { a, b } = pr {
                out.push((a, b));
            }
        }
    }
    out
}

/// 前処理: 縮小 → 極性 → 背景正規化 → 歪み補正 → 二値化。
pub fn prepare(src: &Gray, p: &Params) -> Prepared {
    let (mut g, mut scale) = raster::downscale_to(src, p.max_side);
    if p.rot90 % 4 != 0 {
        g = raster::rot90(&g, p.rot90);
    }
    let inverted = binarize::fix_polarity(&mut g, p.invert);
    // 線が 1 画素ほどしかない低解像度の画像は、細線化や各種判定が不安定になるので 2 倍に拡大して扱う
    if p.upscale_thin && g.w.max(g.h) * 2 <= p.max_side.max(6000) {
        let probe = if binarize::is_clean_scan(&g) && p.source != Source::Blueprint {
            binarize::threshold_global(&g, p.threshold, p.sensitivity)
        } else {
            binarize::threshold(&binarize::flat_field(&g), p.sensitivity)
        };
        let dt = morph::chamfer(&probe, 1, true);
        if morph::stroke_width(&probe, &dt) < 2.0 {
            g = raster::upscale2(&g);
            scale *= 2.0;
        }
    }
    let clean = match p.source {
        Source::Auto => binarize::is_clean_scan(&g),
        Source::Blueprint => false,
        Source::Binary => true,
    };
    let norm0 = if clean {
        g.clone()
    } else {
        binarize::flat_field(&g)
    };
    drop(g);
    let hm: Option<M3> = if let Some(c) = p.corners {
        // 4 隅は元画像 (回転後・縮小前) の座標で受け取る
        let pts = [
            mul(c[0], scale),
            mul(c[1], scale),
            mul(c[2], scale),
            mul(c[3], scale),
        ];
        rectify::from_corners(&pts)
    } else if p.rectify != RectifyMode::None {
        // 推定は縮小した画像で十分
        let (small, s) = raster::downscale_to(&norm0, 2200);
        let bin = binarize_clean(&small, p, clean);
        let segs = rough_segments(&bin);
        rectify::estimate(&segs, small.w, small.h, p.rectify).map(|h| {
            // 縮小座標 → 作業座標に共役変換
            let up = [1.0 / s, 0.0, 0.0, 0.0, 1.0 / s, 0.0, 0.0, 0.0, 1.0];
            let down = [s, 0.0, 0.0, 0.0, s, 0.0, 0.0, 0.0, 1.0];
            m3_mul(&up, &m3_mul(&h, &down))
        })
    } else {
        None
    };
    let (norm, h) = match hm {
        Some(hm) => {
            let (ht, ow, oh) = rectify::fit_canvas(&hm, norm0.w, norm0.h);
            match m3_inv(&ht) {
                Some(inv) => (raster::warp(&norm0, &inv, ow, oh, 255), ht),
                None => (norm0, IDENT),
            }
        }
        None => (norm0, IDENT),
    };
    let bin = binarize_clean(&norm, p, clean);
    Prepared {
        norm,
        bin,
        h,
        inverted,
        clean,
        scale,
    }
}

/// 補正済み二値画像をベクタ化する。
pub fn vectorize(bin_in: &Gray, p: &Params) -> (Document, Stats) {
    let mut bin = bin_in.clone();
    let (w, h) = (bin.w, bin.h);
    let mut stats = Stats {
        width: w,
        height: h,
        ..Default::default()
    };
    let mut doc = Document {
        w: w as f32,
        h: h as f32,
        ..Default::default()
    };

    let dt = morph::chamfer(&bin, 1, true);
    let sw = morph::stroke_width(&bin, &dt);
    stats.stroke_width = sw;
    doc.sw = sw as f32;
    let eps = (p.simplify * sw.max(1.6) * 0.9).max(1.2);

    // --- ベタ塗り ---
    if p.fill {
        let fill = morph::extract_fills(&mut bin, &dt, p.fill_min_width * sw);
        for c in morph::contours(&fill) {
            let mut pts = c;
            pts.push(pts[0]);
            let keep = douglas_peucker(&pts, eps);
            if keep.len() >= 4 {
                let poly: Vec<[f32; 2]> =
                    keep[..keep.len() - 1].iter().map(|&i| v2(pts[i])).collect();
                doc.push(
                    Shape::Poly {
                        pts: poly,
                        closed: true,
                    },
                    Layer::Fill,
                    LineType::Solid,
                    0,
                );
                stats.fills += 1;
            }
        }
    }

    // --- 細線化とチェーン分解 ---
    let (labels, comps) = morph::components(&bin, 1, true);
    let mut sk = skeleton::thin(&bin);
    let chains = skeleton::chains(&mut sk, &dt, &labels);
    drop(sk);
    drop(labels);

    // --- 小さな成分の分類 (文字 / 破線の 1 片) ---
    let text_max = p.text_max_size * sw;
    let mut by_comp: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, c) in chains.iter().enumerate() {
        by_comp.entry(c.comp).or_default().push(i);
    }
    let fo = FitOpts {
        eps,
        sw,
        arcs: p.arcs,
        max_r: 2.0 * w.max(h) as f64,
    };
    let mut kind: HashMap<u32, SmallKind> = HashMap::new();
    if p.text {
        for (&c, idx) in &by_comp {
            let k = &comps[c as usize - 1];
            let size = (k.x1 - k.x0 + 1).max(k.y1 - k.y0 + 1) as f64;
            if size > text_max {
                continue;
            }
            let ch = &chains[idx[0]];
            if idx.len() == 1 && !ch.closed && douglas_peucker(&ch.pts, eps).len() == 2 {
                kind.insert(c, SmallKind::Dash);
            } else if idx.len() == 1
                && ch.closed
                && p.arcs
                && matches!(fit_chain(&ch.pts, true, &fo).as_slice(), [Prim::Circle { r, .. }] if *r > 2.5 * sw)
            {
                // 小さな丸 (通り芯記号・ボルト穴など) は図形として残す
            } else {
                kind.insert(c, SmallKind::Glyph);
            }
        }
    }

    // --- 当てはめ ---
    let mut segs: Vec<Seg> = Vec::new();
    let mut arcs: Vec<(P, f64, f64, f64)> = Vec::new();
    let mut circles: Vec<(P, f64)> = Vec::new();
    let mut polys: Vec<(Vec<P>, bool)> = Vec::new();
    let mut glyph_strokes: HashMap<u32, Vec<Vec<P>>> = HashMap::new();
    let glyph_eps = (eps * 0.6).max(1.0);
    for ch in &chains {
        match kind.get(&ch.comp) {
            Some(SmallKind::Glyph) => {
                let keep = douglas_peucker(&ch.pts, glyph_eps);
                glyph_strokes
                    .entry(ch.comp)
                    .or_default()
                    .push(keep.iter().map(|&i| ch.pts[i]).collect());
                continue;
            }
            Some(SmallKind::Dash) => {
                let (a, b) = (ch.pts[0], ch.pts[ch.pts.len() - 1]);
                segs.push(Seg {
                    a,
                    b,
                    w: ch.width,
                    small: Some(ch.comp),
                    lt: LineType::Solid,
                    hatch: 0,
                    alive: true,
                });
                continue;
            }
            None => {}
        }
        for pr in fit_chain(&ch.pts, ch.closed, &fo) {
            match pr {
                Prim::Seg { a, b } => segs.push(Seg {
                    a,
                    b,
                    w: ch.width,
                    small: None,
                    lt: LineType::Solid,
                    hatch: 0,
                    alive: true,
                }),
                Prim::Arc { c, r, a0, sweep } => arcs.push((c, r, a0, sweep)),
                Prim::Circle { c, r } => circles.push((c, r)),
                Prim::Poly { pts, closed } => polys.push((pts, closed)),
            }
        }
    }
    drop(chains);

    // --- 線分の整理 ---
    let tol = (0.5 * sw).max(1.2);
    let mut consumed = vec![false; comps.len() + 1];
    post::snap_hv(&mut segs, p.snap_hv_deg);
    let ink = |a: P, b: P| -> f64 {
        let n = (dist(a, b).ceil() as usize).max(1);
        let mut hit = 0;
        for i in 0..=n {
            let q = add(a, mul(sub(b, a), i as f64 / n as f64));
            let (x, y) = (q[0].round() as isize, q[1].round() as isize);
            let mut on = false;
            for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (xx, yy) = (x + dx, y + dy);
                if xx >= 0
                    && yy >= 0
                    && (xx as usize) < w
                    && (yy as usize) < h
                    && bin_in.d[yy as usize * w + xx as usize] == 1
                {
                    on = true;
                    break;
                }
            }
            hit += on as usize;
        }
        hit as f64 / (n + 1) as f64
    };
    post::merge_collinear(
        &mut segs,
        (2.0 * sw).max(3.0),
        8.0 * sw,
        tol,
        &mut consumed,
        &ink,
    );
    if p.dashed {
        let o = post::DashOpts {
            max_gap: p.dash_max_gap * sw,
            tol: (0.7 * sw).max(1.5),
            min_parts: 3,
            sw,
        };
        post::merge_dashed(&mut segs, &o, &mut consumed);
    }
    post::snap_hv(&mut segs, p.snap_hv_deg);

    // --- 文字 ---
    if p.text {
        let mut small: Vec<(u32, SmallKind)> = kind
            .iter()
            .filter(|(c, k)| **k == SmallKind::Glyph || !consumed[**c as usize])
            .map(|(c, k)| (*c, *k))
            .collect();
        small.sort_by_key(|s| s.0);
        let clusters = post::cluster_text(&comps, &small, sw, text_max);
        let mut seg_of: HashMap<u32, usize> = HashMap::new();
        for (i, s) in segs.iter().enumerate() {
            if let (true, Some(c)) = (s.alive, s.small) {
                seg_of.insert(c, i);
            }
        }
        let mut in_cluster = vec![false; comps.len() + 1];
        for (cl, has_glyph) in &clusters {
            if !has_glyph {
                // 短い直線だけのかたまり: 一直線に並んでいるなら線 (破線の切れ端)、そうでなければ文字 (「二」「ハ」など)
                let ss: Vec<&Seg> = cl
                    .iter()
                    .filter_map(|c| seg_of.get(c))
                    .map(|&i| &segs[i])
                    .collect();
                let Some(longest) = ss
                    .iter()
                    .max_by(|a, b| a.len().partial_cmp(&b.len()).unwrap())
                else {
                    continue;
                };
                let d = longest.dir();
                if ss.len() < 2
                    || ss.iter().all(|s| {
                        line_offset(s.a, longest.a, d).abs() <= 2.0 * tol
                            && line_offset(s.b, longest.a, d).abs() <= 2.0 * tol
                    })
                {
                    continue;
                }
            }
            let mut strokes: Vec<Vec<[f32; 2]>> = Vec::new();
            let mut bb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for &c in cl {
                in_cluster[c as usize] = true;
                let k = &comps[c as usize - 1];
                bb = [
                    bb[0].min(k.x0 as f32),
                    bb[1].min(k.y0 as f32),
                    bb[2].max(k.x1 as f32 + 1.0),
                    bb[3].max(k.y1 as f32 + 1.0),
                ];
                if let Some(ss) = glyph_strokes.get(&c) {
                    strokes.extend(ss.iter().map(|s| s.iter().map(|q| v2(*q)).collect()));
                }
                if let Some(&i) = seg_of.get(&c) {
                    segs[i].alive = false;
                    strokes.push(vec![v2(segs[i].a), v2(segs[i].b)]);
                }
            }
            // 1 個だけの極小の点はゴミとして捨てる
            if cl.len() == 1 && ((bb[2] - bb[0]).max(bb[3] - bb[1]) as f64) < 2.5 * sw + 2.0 {
                continue;
            }
            if strokes.is_empty() {
                continue;
            }
            doc.push(
                Shape::Text {
                    bbox: bb,
                    s: String::new(),
                    strokes,
                },
                Layer::Text,
                LineType::Solid,
                0,
            );
            stats.texts += 1;
        }
    }

    // --- ハッチング ---
    if p.hatch {
        let o = post::HatchOpts {
            max_spacing: p.hatch_max_spacing * sw,
            min_lines: p.hatch_min_lines,
            axis: p.hatch_axis,
            sw,
        };
        stats.hatch_groups = post::detect_hatch(&mut segs, &o);
    }
    post::snap_ends(&mut segs, (1.5 * sw).max(2.5));
    post::snap_hv(&mut segs, p.snap_hv_deg * 0.5);

    // --- 文書に書き出す ---
    let min_len = p.min_len * sw;
    let gbase = doc.next_group;
    for s in segs.iter().filter(|s| s.alive && s.len() >= min_len) {
        let (layer, group) = if s.hatch != 0 {
            (Layer::Hatch, gbase + s.hatch)
        } else {
            (Layer::Line, 0)
        };
        doc.push(
            Shape::Line {
                a: v2(s.a),
                b: v2(s.b),
            },
            layer,
            s.lt,
            group,
        );
        if let Some(e) = doc.ents.last_mut() {
            e.w = s.w as f32;
        }
        if s.hatch != 0 {
            stats.hatch_lines += 1;
        } else {
            stats.lines += 1;
            if s.lt != LineType::Solid {
                stats.dashed += 1;
            }
        }
    }
    doc.next_group = gbase + stats.hatch_groups;

    // 同じ円に載る円弧 (交差で分断された円) は円にまとめる
    let mut used = vec![false; arcs.len()];
    for i in 0..arcs.len() {
        if used[i] {
            continue;
        }
        let (c, r, _, sw_i) = arcs[i];
        let tolc = (2.0 * sw).max(0.08 * r);
        let mut members = vec![i];
        let mut total = sw_i.abs();
        for (j, a) in arcs.iter().enumerate().skip(i + 1) {
            if !used[j] && dist(a.0, c) <= tolc && (a.1 - r).abs() <= tolc {
                members.push(j);
                total += a.3.abs();
            }
        }
        if members.len() > 1 && total >= 5.0 {
            let mut cc = [0.0, 0.0];
            let mut rr = 0.0;
            for &m in &members {
                used[m] = true;
                let wgt = arcs[m].3.abs() / total;
                cc = add(cc, mul(arcs[m].0, wgt));
                rr += arcs[m].1 * wgt;
            }
            circles.push((cc, rr));
        }
    }
    for (i, (c, r, a0, sweep)) in arcs.iter().enumerate() {
        if !used[i] {
            doc.push(
                Shape::Arc {
                    c: v2(*c),
                    r: *r as f32,
                    a0: *a0 as f32,
                    sweep: *sweep as f32,
                },
                Layer::Line,
                LineType::Solid,
                0,
            );
            stats.arcs += 1;
        }
    }
    for (c, r) in circles {
        doc.push(
            Shape::Circle {
                c: v2(c),
                r: r as f32,
            },
            Layer::Line,
            LineType::Solid,
            0,
        );
        stats.arcs += 1;
    }
    for (pts, closed) in polys {
        doc.push(
            Shape::Poly {
                pts: pts.into_iter().map(v2).collect(),
                closed,
            },
            Layer::Line,
            LineType::Solid,
            0,
        );
        stats.lines += 1;
    }
    (doc, stats)
}

/// 画像 1 枚ぶんの作業状態 (CLI とブラウザ版で共通)。
pub struct Session {
    pub src: Gray,
    pub prepared: Option<Prepared>,
    pub editor: crate::doc::Editor,
    pub stats: Stats,
}

impl Session {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        Ok(Self::from_gray(raster::decode(bytes)?))
    }
    pub fn from_gray(src: Gray) -> Self {
        Session {
            src,
            prepared: None,
            editor: Default::default(),
            stats: Default::default(),
        }
    }

    /// 補正からトレースまで全部やり直す (手修正は失われる)。
    pub fn process(&mut self, p: &Params) -> &Stats {
        let prep = prepare(&self.src, p);
        let (doc, mut stats) = vectorize(&prep.bin, p);
        stats.inverted = prep.inverted;
        stats.binary_source = prep.clean;
        stats.skew_deg = (-prep.h[3]).atan2(prep.h[0]).to_degrees();
        self.prepared = Some(prep);
        let mut ed = crate::doc::Editor::new(doc);
        if p.fill_as != FillAs::Keep {
            let (converted, made) = ed.fills_to_lines(p.fill_as == FillAs::Center);
            stats.fills -= converted.min(stats.fills);
            stats.lines += made;
            ed.clear_selection();
            ed = crate::doc::Editor::new(ed.doc);
        }
        if p.trim_overshoot > 0.0 {
            ed.trim_whiskers(p.trim_overshoot * stats.stroke_width, false);
            ed = crate::doc::Editor::new(ed.doc);
        }
        self.editor = ed;
        self.stats = stats;
        &self.stats
    }

    /// 補正後の座標を、読み込んだ元画像 (回転後) の座標に戻す。
    pub fn to_source(&self, q: P) -> P {
        let scale = self.prepared.as_ref().map_or(1.0, |x| x.scale);
        let h = self.prepared.as_ref().map(|x| x.h).unwrap_or(IDENT);
        let inv = m3_inv(&h).unwrap_or(IDENT);
        mul(m3_apply(&inv, q), 1.0 / scale)
    }
}
