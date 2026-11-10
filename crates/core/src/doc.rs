//! ベクタ図面のデータモデルと、手修正用の編集操作 (選択・削除・接続・線種変更など)。

use crate::geom::{self, P};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub type V = [f32; 2];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineType {
    Solid,
    Dashed,
    Chain,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Line,
    Hatch,
    Text,
    Fill,
}

impl Layer {
    pub fn bit(self) -> u8 {
        match self {
            Layer::Line => 1,
            Layer::Hatch => 2,
            Layer::Text => 4,
            Layer::Fill => 8,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Shape {
    Line {
        a: V,
        b: V,
    },
    Poly {
        pts: Vec<V>,
        closed: bool,
    },
    Circle {
        c: V,
        r: f32,
    },
    /// 開始角 a0 から sweep [rad] (画像座標系, 符号付き)
    Arc {
        c: V,
        r: f32,
        a0: f32,
        sweep: f32,
    },
    /// 文字ブロック。`s` が空ならトレースしたストロークのまま扱う。
    Text {
        bbox: [f32; 4],
        s: String,
        strokes: Vec<Vec<V>>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entity {
    pub id: u32,
    #[serde(flatten)]
    pub shape: Shape,
    pub layer: Layer,
    pub lt: LineType,
    /// ハッチングのグループ番号 (0 = なし)
    #[serde(default)]
    pub group: u32,
    /// 元画像での線の太さ [px] (手で描いた線など不明な場合は 0)
    #[serde(default)]
    pub w: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Document {
    pub w: f32,
    pub h: f32,
    /// 代表線幅 [px]
    pub sw: f32,
    pub ents: Vec<Entity>,
    #[serde(default)]
    pub next_id: u32,
    #[serde(default)]
    pub next_group: u32,
    /// 実寸を書き入れた寸法線 (寸法による補正の根拠)
    #[serde(default)]
    pub dims: Vec<Dim>,
    /// 寸法による補正の結果
    #[serde(default)]
    pub calib: Calib,
}

/// 2 点間の実寸 [mm] を指定した寸法線。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dim {
    pub id: u32,
    pub a: V,
    pub b: V,
    pub len: f32,
}

/// 寸法による補正の状態。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Calib {
    /// トレース直後の座標 → 現在の座標 の線形変換 [a, b, c, d] (x' = a x + b y, y' = c x + d y)。下絵の表示に使う。
    pub m: [f32; 4],
    /// 図面座標 1 単位あたりの実寸 [mm]。0 なら未補正。
    pub unit: f32,
}

impl Default for Calib {
    fn default() -> Self {
        Calib {
            m: [1.0, 0.0, 0.0, 1.0],
            unit: 0.0,
        }
    }
}

pub(crate) fn p(v: V) -> P {
    [v[0] as f64, v[1] as f64]
}
pub(crate) fn v(p: P) -> V {
    [p[0] as f32, p[1] as f32]
}

pub fn arc_points(c: V, r: f32, a0: f32, sweep: f32, step_px: f32) -> Vec<V> {
    let n = ((sweep.abs() * r / step_px.max(0.5)).ceil() as usize).clamp(4, 256);
    (0..=n)
        .map(|i| {
            let a = a0 + sweep * i as f32 / n as f32;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect()
}

impl Shape {
    pub fn bbox(&self) -> [f32; 4] {
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        let mut add = |q: V| {
            b[0] = b[0].min(q[0]);
            b[1] = b[1].min(q[1]);
            b[2] = b[2].max(q[0]);
            b[3] = b[3].max(q[1]);
        };
        match self {
            Shape::Line { a, b } => {
                add(*a);
                add(*b);
            }
            Shape::Poly { pts, .. } => pts.iter().for_each(|q| add(*q)),
            Shape::Circle { c, r } => {
                add([c[0] - r, c[1] - r]);
                add([c[0] + r, c[1] + r]);
            }
            Shape::Arc { c, r, a0, sweep } => arc_points(*c, *r, *a0, *sweep, 4.0)
                .into_iter()
                .for_each(add),
            Shape::Text { bbox, .. } => return *bbox,
        }
        b
    }

    /// 点までの距離。
    pub fn dist(&self, q: P) -> f64 {
        match self {
            Shape::Line { a, b } => geom::dist_seg(q, p(*a), p(*b)),
            Shape::Poly { pts, closed } => {
                let mut d = f64::MAX;
                for w in pts.windows(2) {
                    d = d.min(geom::dist_seg(q, p(w[0]), p(w[1])));
                }
                if *closed && pts.len() > 2 {
                    d = d.min(geom::dist_seg(q, p(pts[pts.len() - 1]), p(pts[0])));
                }
                d
            }
            Shape::Circle { c, r } => (geom::dist(q, p(*c)) - *r as f64).abs(),
            Shape::Arc { c, r, a0, sweep } => {
                let pts = arc_points(*c, *r, *a0, *sweep, 3.0);
                let mut d = f64::MAX;
                for w in pts.windows(2) {
                    d = d.min(geom::dist_seg(q, p(w[0]), p(w[1])));
                }
                d
            }
            Shape::Text { bbox, .. } => {
                let dx = (bbox[0] as f64 - q[0]).max(q[0] - bbox[2] as f64).max(0.0);
                let dy = (bbox[1] as f64 - q[1]).max(q[1] - bbox[3] as f64).max(0.0);
                (dx * dx + dy * dy).sqrt()
            }
        }
    }

    pub fn vertices(&self) -> Vec<V> {
        match self {
            Shape::Line { a, b } => vec![*a, *b],
            Shape::Poly { pts, .. } => pts.clone(),
            Shape::Arc { c, r, a0, sweep } => {
                let e = a0 + sweep;
                vec![
                    [c[0] + r * a0.cos(), c[1] + r * a0.sin()],
                    [c[0] + r * e.cos(), c[1] + r * e.sin()],
                ]
            }
            _ => vec![],
        }
    }

    fn translate(&mut self, dx: f32, dy: f32) {
        let mv = |q: &mut V| {
            q[0] += dx;
            q[1] += dy;
        };
        match self {
            Shape::Line { a, b } => {
                mv(a);
                mv(b);
            }
            Shape::Poly { pts, .. } => pts.iter_mut().for_each(mv),
            Shape::Circle { c, .. } | Shape::Arc { c, .. } => mv(c),
            Shape::Text { bbox, strokes, .. } => {
                bbox[0] += dx;
                bbox[2] += dx;
                bbox[1] += dy;
                bbox[3] += dy;
                strokes.iter_mut().for_each(|s| s.iter_mut().for_each(mv));
            }
        }
    }
}

impl Document {
    pub fn push(&mut self, shape: Shape, layer: Layer, lt: LineType, group: u32) -> u32 {
        self.next_id += 1;
        self.ents.push(Entity {
            id: self.next_id,
            shape,
            layer,
            lt,
            group,
            w: 0.0,
        });
        self.next_id
    }
    pub fn new_group(&mut self) -> u32 {
        self.next_group += 1;
        self.next_group
    }
}

fn seg_hits_rect(a: P, b: P, r: [f64; 4]) -> bool {
    let inside = |q: P| q[0] >= r[0] && q[0] <= r[2] && q[1] >= r[1] && q[1] <= r[3];
    if inside(a) || inside(b) {
        return true;
    }
    let corners = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
    let d = geom::sub(b, a);
    for i in 0..4 {
        let (c0, c1) = (corners[i], corners[(i + 1) % 4]);
        let e = geom::sub(c1, c0);
        let den = geom::cross(d, e);
        if den.abs() < 1e-12 {
            continue;
        }
        let t = geom::cross(geom::sub(c0, a), e) / den;
        let u = geom::cross(geom::sub(c0, a), d) / den;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            return true;
        }
    }
    false
}

const MAX_UNDO: usize = 60;

#[derive(Default)]
pub struct Editor {
    pub doc: Document,
    pub sel: BTreeSet<u32>,
    undo: Vec<Document>,
    redo: Vec<Document>,
    /// なぞり操作中: 最初の変更だけ Undo 履歴に積む
    pub(crate) grouping: bool,
    pub(crate) group_saved: bool,
    /// なぞり操作中にすでに切り詰めた線 (同じ線を 1 回のなぞりで何度も削らない)
    pub(crate) group_touched: Vec<u32>,
}

impl Editor {
    pub fn new(doc: Document) -> Self {
        Editor {
            doc,
            ..Default::default()
        }
    }

    pub(crate) fn checkpoint(&mut self) {
        if self.grouping {
            if self.group_saved {
                return;
            }
            self.group_saved = true;
        }
        self.undo.push(self.doc.clone());
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(d) => {
                self.redo.push(std::mem::replace(&mut self.doc, d));
                self.sel.clear();
                true
            }
            None => false,
        }
    }
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(d) => {
                self.undo.push(std::mem::replace(&mut self.doc, d));
                self.sel.clear();
                true
            }
            None => false,
        }
    }

    fn get(&self, id: u32) -> Option<&Entity> {
        self.doc.ents.iter().find(|e| e.id == id)
    }

    /// 点に最も近い要素。`mask` は対象レイヤのビット和。
    pub fn pick(&self, q: P, tol: f64, mask: u8) -> Option<u32> {
        let mut best = (tol, None);
        for e in &self.doc.ents {
            if e.layer.bit() & mask == 0 {
                continue;
            }
            let bb = e.shape.bbox();
            if q[0] < bb[0] as f64 - tol
                || q[0] > bb[2] as f64 + tol
                || q[1] < bb[1] as f64 - tol
                || q[1] > bb[3] as f64 + tol
            {
                continue;
            }
            let mut d = e.shape.dist(q);
            if matches!(e.shape, Shape::Text { .. }) {
                // 文字ブロックは枠内ならどこでも拾えるが、線を優先する
                d = if d <= 0.0 { tol * 0.8 } else { f64::MAX };
            }
            if d <= best.0 {
                best = (d, Some(e.id));
            }
        }
        best.1
    }

    /// クリック選択。`group` が真ならハッチングのグループごと選ぶ。
    pub fn click(&mut self, q: P, tol: f64, mask: u8, additive: bool, group: bool) -> bool {
        let hit = self.pick(q, tol, mask);
        if !additive {
            self.sel.clear();
        }
        let Some(id) = hit else { return false };
        let g = self.get(id).map(|e| e.group).unwrap_or(0);
        let ids: Vec<u32> = if group && g != 0 {
            self.doc
                .ents
                .iter()
                .filter(|e| e.group == g)
                .map(|e| e.id)
                .collect()
        } else {
            vec![id]
        };
        let all_in = ids.iter().all(|i| self.sel.contains(i));
        for i in ids {
            if additive && all_in {
                self.sel.remove(&i);
            } else {
                self.sel.insert(i);
            }
        }
        true
    }

    /// 矩形選択。`crossing` が真なら矩形に触れたものも選ぶ (偽なら完全に含まれるもののみ)。
    pub fn select_rect(&mut self, r: [f64; 4], mask: u8, additive: bool, crossing: bool) {
        if !additive {
            self.sel.clear();
        }
        for e in &self.doc.ents {
            if e.layer.bit() & mask == 0 {
                continue;
            }
            let bb = e.shape.bbox();
            let (b0, b1, b2, b3) = (bb[0] as f64, bb[1] as f64, bb[2] as f64, bb[3] as f64);
            let inside = b0 >= r[0] && b2 <= r[2] && b1 >= r[1] && b3 <= r[3];
            let hit = if inside {
                true
            } else if !crossing || b2 < r[0] || b0 > r[2] || b3 < r[1] || b1 > r[3] {
                false
            } else {
                match &e.shape {
                    Shape::Line { a, b } => seg_hits_rect(p(*a), p(*b), r),
                    Shape::Poly { pts, .. } => {
                        pts.windows(2).any(|w| seg_hits_rect(p(w[0]), p(w[1]), r))
                    }
                    Shape::Arc {
                        c,
                        r: rr,
                        a0,
                        sweep,
                    } => arc_points(*c, *rr, *a0, *sweep, 3.0)
                        .windows(2)
                        .any(|w| seg_hits_rect(p(w[0]), p(w[1]), r)),
                    Shape::Circle { c, r: rr } => {
                        arc_points(*c, *rr, 0.0, std::f32::consts::TAU, 3.0)
                            .windows(2)
                            .any(|w| seg_hits_rect(p(w[0]), p(w[1]), r))
                    }
                    Shape::Text { .. } => true,
                }
            };
            if hit {
                self.sel.insert(e.id);
            }
        }
    }

    pub fn select_all(&mut self, mask: u8) {
        self.sel = self
            .doc
            .ents
            .iter()
            .filter(|e| e.layer.bit() & mask != 0)
            .map(|e| e.id)
            .collect();
    }
    pub fn clear_selection(&mut self) {
        self.sel.clear();
    }

    pub fn delete_selected(&mut self) -> usize {
        if self.sel.is_empty() {
            return 0;
        }
        self.checkpoint();
        let n = self.sel.len();
        let sel = std::mem::take(&mut self.sel);
        self.doc.ents.retain(|e| !sel.contains(&e.id));
        n
    }

    pub fn set_linetype(&mut self, lt: LineType) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        for e in self
            .doc
            .ents
            .iter_mut()
            .filter(|e| self.sel.contains(&e.id))
        {
            e.lt = lt;
        }
    }

    /// 選択要素のレイヤを変える (ハッチング扱いにする / 通常の線に戻す 等)。
    pub fn set_layer(&mut self, layer: Layer) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        let g = if layer == Layer::Hatch {
            self.doc.new_group()
        } else {
            0
        };
        for e in self
            .doc
            .ents
            .iter_mut()
            .filter(|e| self.sel.contains(&e.id))
        {
            if matches!(e.shape, Shape::Text { .. }) {
                continue;
            }
            e.layer = layer;
            e.group = g;
        }
    }

    pub fn translate_selected(&mut self, dx: f32, dy: f32) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        for e in self
            .doc
            .ents
            .iter_mut()
            .filter(|e| self.sel.contains(&e.id))
        {
            e.shape.translate(dx, dy);
        }
    }

    /// 選択した線分を接続する。
    /// - ほぼ同一直線上の線 (破線の切れ端など) は 1 本の線にまとめる
    /// - 平行でない 2 本だけを選んだ場合は交点まで延長/切り詰めて角を作る
    ///
    /// 戻り値は作られた接続の数。
    pub fn join_selected(&mut self) -> usize {
        let lines: Vec<(u32, P, P, LineType)> = self
            .doc
            .ents
            .iter()
            .filter(|e| self.sel.contains(&e.id))
            .filter_map(|e| match e.shape {
                Shape::Line { a, b } => Some((e.id, p(a), p(b), e.lt)),
                _ => None,
            })
            .collect();
        if lines.len() < 2 {
            return 0;
        }
        let tol0 = (self.doc.sw as f64 * 3.0).max(4.0);
        // 同一直線ごとにまとめる
        let mut order: Vec<usize> = (0..lines.len()).collect();
        order.sort_by(|&i, &j| {
            geom::dist(lines[j].1, lines[j].2)
                .partial_cmp(&geom::dist(lines[i].1, lines[i].2))
                .unwrap()
        });
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        for &i in &order {
            let mut placed = false;
            for c in clusters.iter_mut() {
                let (_, a, b, _) = lines[c[0]];
                let d = geom::unit(geom::sub(b, a));
                let tol = tol0.max(0.02 * geom::dist(a, b));
                if geom::line_offset(lines[i].1, a, d).abs() <= tol
                    && geom::line_offset(lines[i].2, a, d).abs() <= tol
                {
                    c.push(i);
                    placed = true;
                    break;
                }
            }
            if !placed {
                clusters.push(vec![i]);
            }
        }
        if lines.len() == 2 && clusters.len() == 2 {
            let (i0, a0, b0, _) = lines[0];
            let (i1, a1, b1, _) = lines[1];
            let Some(x) = geom::line_intersect(
                a0,
                geom::unit(geom::sub(b0, a0)),
                a1,
                geom::unit(geom::sub(b1, a1)),
            ) else {
                return 0;
            };
            self.checkpoint();
            for (id, a, b) in [(i0, a0, b0), (i1, a1, b1)] {
                let e = self.doc.ents.iter_mut().find(|e| e.id == id).unwrap();
                e.shape = if geom::dist(a, x) < geom::dist(b, x) {
                    Shape::Line { a: v(x), b: v(b) }
                } else {
                    Shape::Line { a: v(a), b: v(x) }
                };
            }
            return 1;
        }
        let merge: Vec<&Vec<usize>> = clusters.iter().filter(|c| c.len() >= 2).collect();
        if merge.is_empty() {
            return 0;
        }
        self.checkpoint();
        let n = merge.len();
        for c in merge {
            let mut pts = Vec::new();
            let mut wts = Vec::new();
            let mut lt = LineType::Solid;
            let mut sum_len = 0.0;
            for &i in c {
                let (_, a, b, l) = lines[i];
                let w = geom::dist(a, b).max(1.0);
                pts.push(a);
                pts.push(b);
                wts.push(w);
                wts.push(w);
                sum_len += w;
                if l != LineType::Solid {
                    lt = l;
                }
            }
            let (c0, d) = geom::fit_line(&pts, Some(&wts));
            let (mut t0, mut t1) = (f64::MAX, f64::MIN);
            for q in &pts {
                let t = geom::dot(geom::sub(*q, c0), d);
                t0 = t0.min(t);
                t1 = t1.max(t);
            }
            if lt == LineType::Solid && c.len() >= 3 && sum_len < 0.85 * (t1 - t0) {
                lt = LineType::Dashed;
            }
            let ids: BTreeSet<u32> = c.iter().map(|&i| lines[i].0).collect();
            self.doc.ents.retain(|e| !ids.contains(&e.id));
            for i in &ids {
                self.sel.remove(i);
            }
            let id = self.doc.push(
                Shape::Line {
                    a: v(geom::add(c0, geom::mul(d, t0))),
                    b: v(geom::add(c0, geom::mul(d, t1))),
                },
                Layer::Line,
                lt,
                0,
            );
            self.sel.insert(id);
        }
        n
    }

    /// 選択した線・折れ線を 1 つの文字ブロックにまとめる。
    pub fn selection_to_text(&mut self) -> bool {
        let mut strokes: Vec<Vec<V>> = Vec::new();
        let mut ids = BTreeSet::new();
        for e in self.doc.ents.iter().filter(|e| self.sel.contains(&e.id)) {
            match &e.shape {
                Shape::Line { a, b } => strokes.push(vec![*a, *b]),
                Shape::Poly { pts, closed } => {
                    let mut s = pts.clone();
                    if *closed && !s.is_empty() {
                        s.push(s[0]);
                    }
                    strokes.push(s)
                }
                Shape::Arc { c, r, a0, sweep } => {
                    strokes.push(arc_points(*c, *r, *a0, *sweep, 2.0))
                }
                Shape::Circle { c, r } => {
                    strokes.push(arc_points(*c, *r, 0.0, std::f32::consts::TAU, 2.0))
                }
                Shape::Text { strokes: s, .. } => strokes.extend(s.iter().cloned()),
            }
            ids.insert(e.id);
        }
        if strokes.is_empty() {
            return false;
        }
        self.checkpoint();
        let mut bb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for q in strokes.iter().flatten() {
            bb = [
                bb[0].min(q[0]),
                bb[1].min(q[1]),
                bb[2].max(q[0]),
                bb[3].max(q[1]),
            ];
        }
        self.doc.ents.retain(|e| !ids.contains(&e.id));
        self.sel.clear();
        let id = self.doc.push(
            Shape::Text {
                bbox: bb,
                s: String::new(),
                strokes,
            },
            Layer::Text,
            LineType::Solid,
            0,
        );
        self.sel.insert(id);
        true
    }

    /// 選択した文字ブロックをストロークのまま通常の線に戻す。
    pub fn text_to_lines(&mut self) -> bool {
        let targets: Vec<u32> = self
            .doc
            .ents
            .iter()
            .filter(|e| self.sel.contains(&e.id) && matches!(e.shape, Shape::Text { .. }))
            .map(|e| e.id)
            .collect();
        if targets.is_empty() {
            return false;
        }
        self.checkpoint();
        for id in targets {
            let i = self.doc.ents.iter().position(|e| e.id == id).unwrap();
            let e = self.doc.ents.remove(i);
            self.sel.remove(&id);
            if let Shape::Text { strokes, .. } = e.shape {
                for s in strokes {
                    let nid = if s.len() == 2 {
                        self.doc.push(
                            Shape::Line { a: s[0], b: s[1] },
                            Layer::Line,
                            LineType::Solid,
                            0,
                        )
                    } else {
                        self.doc.push(
                            Shape::Poly {
                                pts: s,
                                closed: false,
                            },
                            Layer::Line,
                            LineType::Solid,
                            0,
                        )
                    };
                    self.sel.insert(nid);
                }
            }
        }
        true
    }

    pub fn add_line(&mut self, a: P, b: P, lt: LineType) -> u32 {
        self.checkpoint();
        self.doc
            .push(Shape::Line { a: v(a), b: v(b) }, Layer::Line, lt, 0)
    }
    pub fn add_circle(&mut self, c: P, r: f64) -> u32 {
        self.checkpoint();
        self.doc.push(
            Shape::Circle {
                c: v(c),
                r: r as f32,
            },
            Layer::Line,
            LineType::Solid,
            0,
        )
    }
    pub fn add_text(&mut self, pos: P, h: f64, s: &str) -> u32 {
        self.checkpoint();
        let wdt = h * s.chars().count().max(1) as f64;
        let bbox = [
            pos[0] as f32,
            (pos[1] - h) as f32,
            (pos[0] + wdt) as f32,
            pos[1] as f32,
        ];
        self.doc.push(
            Shape::Text {
                bbox,
                s: s.to_string(),
                strokes: vec![],
            },
            Layer::Text,
            LineType::Solid,
            0,
        )
    }
    pub fn set_text(&mut self, id: u32, s: &str) -> bool {
        if !self
            .doc
            .ents
            .iter()
            .any(|e| e.id == id && matches!(e.shape, Shape::Text { .. }))
        {
            return false;
        }
        self.checkpoint();
        if let Some(Entity {
            shape: Shape::Text { s: t, .. },
            ..
        }) = self.doc.ents.iter_mut().find(|e| e.id == id)
        {
            *t = s.to_string();
        }
        true
    }

    /// 選択中の要素の頂点のうち、点に最も近いもの (要素 id, 頂点番号)。
    pub fn pick_vertex(&self, q: P, tol: f64) -> Option<(u32, usize)> {
        let mut best = (tol, None);
        for e in self.doc.ents.iter().filter(|e| self.sel.contains(&e.id)) {
            if matches!(e.shape, Shape::Arc { .. }) {
                continue;
            }
            for (i, vv) in e.shape.vertices().iter().enumerate() {
                let d = geom::dist(q, p(*vv));
                if d <= best.0 {
                    best = (d, Some((e.id, i)));
                }
            }
        }
        best.1
    }

    pub fn move_vertex(&mut self, id: u32, idx: usize, q: P) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        self.checkpoint();
        let e = self.doc.ents.iter_mut().find(|e| e.id == id).unwrap();
        match &mut e.shape {
            Shape::Line { a, b } => {
                if idx == 0 {
                    *a = v(q)
                } else {
                    *b = v(q)
                }
            }
            Shape::Poly { pts, .. } if idx < pts.len() => pts[idx] = v(q),
            _ => return false,
        }
        true
    }

    /// スナップ先を探す。端点 → 交点 → 線上の最近点 の優先順。
    /// 戻り値 (点, 種類: 1=端点, 2=交点, 3=線上)。
    pub fn snap(&self, q: P, tol: f64, mask: u8, exclude: u32) -> Option<(P, u8)> {
        let mut best_end = (tol, None);
        let mut near: Vec<(P, P)> = Vec::new();
        let mut best_on = (tol, None);
        for e in &self.doc.ents {
            if e.layer.bit() & mask == 0 || e.id == exclude || matches!(e.shape, Shape::Text { .. })
            {
                continue;
            }
            let bb = e.shape.bbox();
            if q[0] < bb[0] as f64 - tol
                || q[0] > bb[2] as f64 + tol
                || q[1] < bb[1] as f64 - tol
                || q[1] > bb[3] as f64 + tol
            {
                continue;
            }
            for vv in e.shape.vertices() {
                let d = geom::dist(q, p(vv));
                if d < best_end.0 {
                    best_end = (d, Some(p(vv)));
                }
            }
            if let Shape::Circle { c, .. } = e.shape {
                let d = geom::dist(q, p(c));
                if d < best_end.0 {
                    best_end = (d, Some(p(c)));
                }
            }
            if let Shape::Line { a, b } = e.shape {
                let (a, b) = (p(a), p(b));
                let d = geom::dist_seg(q, a, b);
                if d <= tol {
                    near.push((a, b));
                    if d < best_on.0 {
                        let ab = geom::sub(b, a);
                        let t = (geom::dot(geom::sub(q, a), ab) / geom::dot(ab, ab).max(1e-9))
                            .clamp(0.0, 1.0);
                        best_on = (d, Some(geom::add(a, geom::mul(ab, t))));
                    }
                }
            }
        }
        if let Some(e) = best_end.1 {
            return Some((e, 1));
        }
        let mut best_x = (tol, None);
        for i in 0..near.len() {
            for j in i + 1..near.len() {
                let (a0, b0) = near[i];
                let (a1, b1) = near[j];
                if let Some(x) = geom::line_intersect(
                    a0,
                    geom::unit(geom::sub(b0, a0)),
                    a1,
                    geom::unit(geom::sub(b1, a1)),
                ) {
                    let d = geom::dist(x, q);
                    if d < best_x.0 {
                        best_x = (d, Some(x));
                    }
                }
            }
        }
        if let Some(x) = best_x.1 {
            return Some((x, 2));
        }
        best_on.1.map(|o| (o, 3))
    }

    /// 点に最も近い線分をその位置で 2 本に分割する。
    pub fn split_at(&mut self, q: P, tol: f64, mask: u8) -> bool {
        let Some(id) = self.pick(q, tol, mask) else {
            return false;
        };
        let Some(e) = self.get(id).cloned() else {
            return false;
        };
        let Shape::Line { a, b } = e.shape else {
            return false;
        };
        let (pa, pb) = (p(a), p(b));
        let ab = geom::sub(pb, pa);
        let t = geom::dot(geom::sub(q, pa), ab) / geom::dot(ab, ab).max(1e-9);
        if !(0.02..=0.98).contains(&t) {
            return false;
        }
        let m = v(geom::add(pa, geom::mul(ab, t)));
        self.checkpoint();
        let ent = self.doc.ents.iter_mut().find(|x| x.id == id).unwrap();
        ent.shape = Shape::Line { a, b: m };
        let nid = self
            .doc
            .push(Shape::Line { a: m, b }, e.layer, e.lt, e.group);
        self.sel.clear();
        self.sel.insert(id);
        self.sel.insert(nid);
        true
    }
}
