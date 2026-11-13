//! 手修正の仕上げ用の操作: ごみ消しペン、種別変更ペン、切れた端点の検出、
//! わずかな傾きの一括修正、選択した線から円・楕円・矩形への整形。

use crate::doc::{p, v, Editor, Layer, LineType, Nodes, Shape, LINK_TOL, V};
use crate::geom::{self, P};
use std::collections::{BTreeSet, HashMap};

fn to_path(path: &[P], q: P) -> f64 {
    if path.len() == 1 {
        return geom::dist(q, path[0]);
    }
    path.windows(2)
        .map(|w| geom::dist_seg(q, w[0], w[1]))
        .fold(f64::MAX, f64::min)
}

/// 折れ線の集まりを `step` 以下の間隔で標本化する。
fn sample(polys: &[Vec<V>], step: f64) -> Vec<P> {
    let mut out = Vec::new();
    for s in polys {
        if s.len() == 1 {
            out.push(p(s[0]));
        }
        for w in s.windows(2) {
            let (a, b) = (p(w[0]), p(w[1]));
            let n = ((geom::dist(a, b) / step.max(0.25)).ceil() as usize).clamp(1, 400);
            for k in 0..=n {
                out.push(geom::add(
                    a,
                    geom::mul(geom::sub(b, a), k as f64 / n as f64),
                ));
            }
        }
    }
    out
}

/// n 元連立一次方程式 (ガウスの消去法)。
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for c in 0..n {
        let piv = (c..n).max_by(|&x, &y| a[x][c].abs().partial_cmp(&a[y][c].abs()).unwrap())?;
        if a[piv][c].abs() < 1e-10 {
            return None;
        }
        a.swap(c, piv);
        b.swap(c, piv);
        for r in 0..n {
            if r != c {
                let f = a[r][c] / a[c][c];
                for q in c..n {
                    a[r][q] -= f * a[c][q];
                }
                b[r] -= f * b[c];
            }
        }
    }
    Some((0..n).map(|i| b[i] / a[i][i]).collect())
}

/// 点列への楕円の当てはめ (一般の二次曲線の最小二乗)。(中心, rx, ry, 回転)。
fn fit_ellipse(pts: &[P]) -> Option<(P, f64, f64, f64)> {
    if pts.len() < 6 {
        return None;
    }
    let n = pts.len() as f64;
    let m = geom::mul(
        pts.iter().fold([0.0, 0.0], |s, q| geom::add(s, *q)),
        1.0 / n,
    );
    let sc = (pts.iter().map(|q| geom::dist(*q, m)).sum::<f64>() / n).max(1e-9);
    let mut ata = vec![vec![0.0; 5]; 5];
    let mut atb = vec![0.0; 5];
    for q in pts {
        let (x, y) = ((q[0] - m[0]) / sc, (q[1] - m[1]) / sc);
        let row = [x * x, x * y, y * y, x, y];
        for i in 0..5 {
            for j in 0..5 {
                ata[i][j] += row[i] * row[j];
            }
            atb[i] += row[i];
        }
    }
    let s = solve(ata, atb)?;
    let (a, b, c, d, e) = (s[0], s[1], s[2], s[3], s[4]);
    if b * b - 4.0 * a * c >= -1e-9 {
        return None;
    }
    let ctr = solve(vec![vec![2.0 * a, b], vec![b, 2.0 * c]], vec![-d, -e])?;
    let (x0, y0) = (ctr[0], ctr[1]);
    let f = a * x0 * x0 + b * x0 * y0 + c * y0 * y0 + d * x0 + e * y0 - 1.0;
    let th = 0.5 * b.atan2(a - c);
    let (cs, sn) = (th.cos(), th.sin());
    let l1 = a * cs * cs + b * cs * sn + c * sn * sn;
    let l2 = a * sn * sn - b * cs * sn + c * cs * cs;
    let (r1, r2) = (-f / l1, -f / l2);
    if !(r1 > 0.0 && r2 > 0.0) || !r1.is_finite() || !r2.is_finite() {
        return None;
    }
    // rx を長軸にし、回転は (-90°, 90°] に収める
    let (mut rx, mut ry, mut rot) = (r1.sqrt() * sc, r2.sqrt() * sc, th);
    if rx < ry {
        std::mem::swap(&mut rx, &mut ry);
        rot += std::f64::consts::FRAC_PI_2;
    }
    let pi = std::f64::consts::PI;
    rot -= ((rot + pi / 2.0) / pi).ceil() * pi - pi;
    Some(([m[0] + x0 * sc, m[1] + y0 * sc], rx, ry, rot))
}

/// 整形した図形。
enum Fit {
    Circle(P, f64),
    Ellipse(P, f64, f64, f64),
    /// 4 隅 (左上・右上・右下・左下の順。回転している場合はその枠内での並び)
    Rect([P; 4]),
}

impl Fit {
    /// 点が図形の内側 (境界から `tol` 外側まで) にあるか。
    fn contains(&self, q: P, tol: f64) -> bool {
        match self {
            Fit::Circle(c, r) => geom::dist(q, *c) <= r + tol,
            Fit::Ellipse(c, rx, ry, rot) => {
                let d = geom::sub(q, *c);
                let (x, y) = (
                    d[0] * rot.cos() + d[1] * rot.sin(),
                    -d[0] * rot.sin() + d[1] * rot.cos(),
                );
                (x / (rx + tol)).powi(2) + (y / (ry + tol)).powi(2) <= 1.0
            }
            Fit::Rect(c) => (0..4).all(|i| {
                let d = geom::unit(geom::sub(c[(i + 1) % 4], c[i]));
                geom::line_offset(q, c[i], d) >= -tol
            }),
        }
    }
}

impl Editor {
    // ---------------- ペンでなぞった要素の選択 ----------------

    /// ペンでなぞった経路 `path` (半径 `r`) に触れた要素を選び直す (向きは問わない)。
    ///
    /// - `min_cover`: 要素の長さのうち、この割合以上がなぞった範囲に入っているものだけを選ぶ
    ///   (0 なら少しでも触れたもの)。長い線を横切っただけでは選ばれないようにするため。
    /// - `max_len`: 0 より大きければ、この長さ以下の要素だけを選ぶ。
    pub fn select_touching_stroke(
        &mut self,
        path: &[P],
        r: f64,
        mask: u8,
        min_cover: f64,
        max_len: f64,
    ) {
        self.sel.clear();
        if path.is_empty() {
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for q in path {
            x0 = x0.min(q[0] - r);
            y0 = y0.min(q[1] - r);
            x1 = x1.max(q[0] + r);
            y1 = y1.max(q[1] + r);
        }
        for e in &self.doc.ents {
            if e.layer.bit() & mask == 0 {
                continue;
            }
            let bb = e.shape.bbox();
            if (bb[2] as f64) < x0
                || (bb[0] as f64) > x1
                || (bb[3] as f64) < y0
                || (bb[1] as f64) > y1
            {
                continue;
            }
            if max_len > 0.0 {
                let len = match &e.shape {
                    // 文字ブロックは枠の対角線の長さで比べる
                    Shape::Text { bbox, .. } => {
                        ((bbox[2] - bbox[0]) as f64).hypot((bbox[3] - bbox[1]) as f64)
                    }
                    s => s.length(),
                };
                if len > max_len {
                    continue;
                }
            }
            let pts = sample(&e.shape.polylines(2.0), (r * 0.5).max(0.5));
            if pts.is_empty() {
                continue;
            }
            let inside = pts.iter().filter(|q| to_path(path, **q) <= r).count();
            let cover = inside as f64 / pts.len() as f64;
            if inside > 0 && cover >= min_cover {
                self.sel.insert(e.id);
            }
        }
    }

    /// ごみ消しペン: 経路に触れた、長さ `max_len` 以下の要素を消す。戻り値は消した数。
    pub fn erase_stroke(&mut self, path: &[P], r: f64, max_len: f64, mask: u8) -> usize {
        let keep = std::mem::take(&mut self.sel);
        self.select_touching_stroke(path, r, mask, 0.0, max_len.max(1e-6));
        let n = self.delete_selected();
        self.sel = keep;
        let alive: BTreeSet<u32> = self.doc.ents.iter().map(|e| e.id).collect();
        self.sel.retain(|id| alive.contains(id));
        n
    }

    /// 選択要素の種別を変える。全体が 1 回の Undo で戻る。戻り値は変更した要素の数。
    ///
    /// kind: 0 = 実線, 1 = 破線, 2 = 一点鎖線, 3 = ハッチング, 4 = 文字。
    /// 文字ブロックを線・ハッチングにする場合は、先にストロークへ分解する。
    pub fn retype_selected(&mut self, kind: u8) -> usize {
        let n = self.sel.len();
        if n == 0 {
            return 0;
        }
        let outer = self.grouping;
        if !outer {
            self.grouping = true;
            self.group_saved = false;
        }
        if kind == 4 {
            self.selection_to_text();
        } else {
            self.text_to_lines();
            if kind == 3 {
                self.set_layer(Layer::Hatch);
            } else {
                self.set_layer(Layer::Line);
                self.set_linetype(match kind {
                    1 => LineType::Dashed,
                    2 => LineType::Chain,
                    _ => LineType::Solid,
                });
            }
        }
        if !outer {
            self.grouping = false;
            self.group_saved = false;
        }
        n
    }

    // ---------------- 切れた端点 ----------------

    /// どこにもつながっていない端点 (線・開いた折れ線・円弧の先) を返す。
    /// 途切れた線を見つけるための表示用。対象は線レイヤで、線・塗りレイヤの要素に
    /// 触れていない端点だけを返す。
    pub fn free_ends(&self) -> Vec<V> {
        let tol = (self.doc.sw as f64 * 0.75).max(1.0);
        const CELL: f64 = 48.0;
        let key = |q: P| ((q[0] / CELL).floor() as i64, (q[1] / CELL).floor() as i64);
        // 線分 (所属要素の番号つき) を格子に登録する
        let mut segs: Vec<(usize, P, P)> = Vec::new();
        let mut curves: Vec<usize> = Vec::new();
        for (i, e) in self.doc.ents.iter().enumerate() {
            if e.layer != Layer::Line && e.layer != Layer::Fill {
                continue;
            }
            match &e.shape {
                Shape::Line { .. } | Shape::Poly { .. } => {
                    for s in e.shape.polylines(3.0) {
                        for w in s.windows(2) {
                            segs.push((i, p(w[0]), p(w[1])));
                        }
                    }
                }
                Shape::Text { .. } => {}
                _ => curves.push(i),
            }
        }
        let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (k, (_, a, b)) in segs.iter().enumerate() {
            let n = (geom::dist(*a, *b) / (CELL * 0.5)).ceil().max(1.0) as usize;
            let mut last = None;
            for t in 0..=n {
                let c = key(geom::add(
                    *a,
                    geom::mul(geom::sub(*b, *a), t as f64 / n as f64),
                ));
                if last != Some(c) {
                    grid.entry(c).or_default().push(k);
                    last = Some(c);
                }
            }
        }
        let mut out = Vec::new();
        for (i, e) in self.doc.ents.iter().enumerate() {
            if e.layer != Layer::Line {
                continue;
            }
            let ends: Vec<V> = match &e.shape {
                Shape::Line { a, b } => vec![*a, *b],
                Shape::Poly { pts, closed: false } if pts.len() >= 2 => {
                    vec![pts[0], pts[pts.len() - 1]]
                }
                Shape::Arc { .. } => e.shape.vertices(),
                _ => continue,
            };
            // 両端が同じ点 (閉じた形) なら端点ではない
            if ends.len() == 2 && geom::dist(p(ends[0]), p(ends[1])) <= tol {
                continue;
            }
            for q in ends {
                let qp = p(q);
                let (cx, cy) = key(qp);
                let mut connected = false;
                'cells: for dx in -1..=1 {
                    for dy in -1..=1 {
                        let Some(c) = grid.get(&(cx + dx, cy + dy)) else {
                            continue;
                        };
                        for &k in c {
                            let (owner, a, b) = segs[k];
                            if owner != i && geom::dist_seg(qp, a, b) <= tol {
                                connected = true;
                                break 'cells;
                            }
                        }
                    }
                }
                if !connected {
                    connected = curves.iter().any(|&j| {
                        j != i && {
                            let bb = self.doc.ents[j].shape.bbox();
                            qp[0] >= bb[0] as f64 - tol
                                && qp[0] <= bb[2] as f64 + tol
                                && qp[1] >= bb[1] as f64 - tol
                                && qp[1] <= bb[3] as f64 + tol
                                && self.doc.ents[j].shape.dist(qp) <= tol
                        }
                    });
                }
                if !connected {
                    out.push(q);
                }
            }
        }
        out
    }

    // ---------------- わずかな傾きの修正 ----------------

    /// ほぼ水平・垂直なのにわずかに傾いている線を、厳密な水平・垂直にそろえる。
    ///
    /// 水平・垂直からのずれが `max_deg` 度以内の線 (折れ線の辺) が対象。
    /// `only_selected` が真なら選択中の要素だけ、偽なら線レイヤすべて。
    /// 同じ座標にある頂点はつながっているものとして一緒に動かすので、角や接続は外れない。
    /// 戻り値は直した線 (辺) の数。
    pub fn straighten(&mut self, max_deg: f64, only_selected: bool) -> usize {
        let tan = max_deg.clamp(0.0, 30.0).to_radians().tan();
        let mut nodes = Nodes::new(LINK_TOL);
        // 節点ごとの x・y の同値類 (union-find)
        fn find(u: &mut Vec<usize>, i: usize) -> usize {
            let mut r = i;
            while u[r] != r {
                r = u[r];
            }
            let mut k = i;
            while u[k] != r {
                let nx = u[k];
                u[k] = r;
                k = nx;
            }
            r
        }
        // 辺: (節点 a, 節点 b, 軸 0 = 水平にする (y をそろえる) / 1 = 垂直にする, 長さ, 傾いているか)
        let mut edges: Vec<(usize, usize, usize, f64, bool)> = Vec::new();
        for e in &self.doc.ents {
            let target = if only_selected {
                self.sel.contains(&e.id)
            } else {
                e.layer == Layer::Line
            };
            if !target || !matches!(e.shape, Shape::Line { .. } | Shape::Poly { .. }) {
                continue;
            }
            for s in e.shape.polylines(3.0) {
                for w in s.windows(2) {
                    let (a, b) = (p(w[0]), p(w[1]));
                    let (dx, dy) = ((b[0] - a[0]).abs(), (b[1] - a[1]).abs());
                    if dx.max(dy) < 2.0 * LINK_TOL {
                        continue;
                    }
                    let axis = if dy <= dx { 0 } else { 1 };
                    let dev = dx.min(dy);
                    if dev > tan * dx.max(dy) {
                        continue;
                    }
                    let (Some(na), Some(nb)) = (nodes.id(a, true), nodes.id(b, true)) else {
                        continue;
                    };
                    if na != nb {
                        edges.push((na, nb, axis, dx.max(dy), dev > 1e-4));
                    }
                }
            }
        }
        let fixed = edges.iter().filter(|e| e.4).count();
        if fixed == 0 {
            return 0;
        }
        let n = nodes.pts.len();
        // uf[0]: y をそろえる組, uf[1]: x をそろえる組
        let mut uf = [
            (0..n).collect::<Vec<usize>>(),
            (0..n).collect::<Vec<usize>>(),
        ];
        for &(a, b, axis, _, _) in &edges {
            let (ra, rb) = (find(&mut uf[axis], a), find(&mut uf[axis], b));
            if ra != rb {
                uf[axis][ra] = rb;
            }
        }
        // 組ごとの座標: 辺の長さで重みづけした平均。傾いた辺を含まない組は動かさない。
        let mut target: [HashMap<usize, (f64, f64, bool)>; 2] = [HashMap::new(), HashMap::new()];
        for &(a, b, axis, len, tilted) in &edges {
            let r = find(&mut uf[axis], a);
            let c = 1 - axis; // 水平にする辺は y (添字 1) をそろえる
            let t = target[axis].entry(r).or_insert((0.0, 0.0, false));
            t.0 += len * (nodes.pts[a][c] + nodes.pts[b][c]) * 0.5;
            t.1 += len;
            t.2 |= tilted;
        }
        let mut moved: HashMap<usize, P> = HashMap::new();
        for i in 0..n {
            let mut q = nodes.pts[i];
            let mut changed = false;
            for axis in 0..2 {
                let r = find(&mut uf[axis], i);
                if let Some(&(s, w, true)) = target[axis].get(&r) {
                    q[1 - axis] = s / w;
                    changed = true;
                }
            }
            if changed {
                moved.insert(i, q);
            }
        }
        self.checkpoint();
        for e in self.doc.ents.iter_mut() {
            let mut mv = |w: &mut V| {
                if let Some(q) = nodes.id(p(*w), false).and_then(|i| moved.get(&i)) {
                    *w = v(*q);
                }
            };
            match &mut e.shape {
                Shape::Line { a, b } => {
                    mv(a);
                    mv(b);
                }
                Shape::Poly { pts, .. } => pts.iter_mut().for_each(mv),
                _ => {}
            }
        }
        fixed
    }

    // ---------------- 円・楕円・矩形への整形 ----------------

    /// 選択した線・円弧 (一部しか認識されていない円や矩形) を、正円・楕円・矩形に整える。
    ///
    /// - `kind`: 0 = 自動 (選択の中の円・楕円・円弧が主ならその形、そうでなければ矩形),
    ///   1 = 円, 2 = 楕円, 3 = 矩形
    /// - `pattern`: 中に入れる線。0 = なし, 1 = 斜線 1 本 (〼), 2 = 対角の × (☒), 3 = 十字 (田)
    /// - `clear_inside`: 真なら、図形の内側にある要素 (`mask` のレイヤ) をすべて消す
    ///
    /// 選択していた線は新しい図形 (矩形は 4 辺の線) に置き換わる。戻り値は結果の説明。
    pub fn fit_shape(
        &mut self,
        kind: u8,
        pattern: u8,
        clear_inside: bool,
        mask: u8,
    ) -> Result<String, String> {
        let sw = (self.doc.sw as f64).max(1.0);
        let sel: Vec<&crate::doc::Entity> = self
            .doc
            .ents
            .iter()
            .filter(|e| self.sel.contains(&e.id) && !matches!(e.shape, Shape::Text { .. }))
            .collect();
        if sel.is_empty() {
            return Err("整えたい線・円弧を選択してください".into());
        }
        let lt = sel[0].lt;
        let pts: Vec<P> = sel
            .iter()
            .flat_map(|e| sample(&e.shape.polylines(2.0), 3.0))
            .collect();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for q in &pts {
            x0 = x0.min(q[0]);
            y0 = y0.min(q[1]);
            x1 = x1.max(q[0]);
            y1 = y1.max(q[1]);
        }
        let extent = (x1 - x0).max(y1 - y0);
        if extent < 2.0 * sw {
            return Err("選択が小さすぎます".into());
        }
        // 選択の中でいちばん大きい円・楕円・円弧
        let round = sel
            .iter()
            .filter_map(|e| match e.shape {
                Shape::Circle { c, r } => Some((r as f64, Fit::Circle(p(c), r as f64))),
                Shape::Arc { c, r, .. } => Some((r as f64, Fit::Circle(p(c), r as f64))),
                Shape::Ellipse { c, rx, ry, rot } => Some((
                    rx.max(ry) as f64,
                    Fit::Ellipse(p(c), rx as f64, ry as f64, rot as f64),
                )),
                _ => None,
            })
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let only_one = sel.len() == 1;
        let fit = match kind {
            0 => match round {
                Some((r, f)) if 2.0 * r >= 0.6 * extent => f,
                _ => self.fit_rect(&sel, &pts)?,
            },
            1 => match round {
                Some((_, Fit::Circle(c, r))) if only_one => Fit::Circle(c, r),
                _ => {
                    let (c, r) = geom::fit_circle(&pts)
                        .filter(|(_, r)| r.is_finite() && *r >= sw && *r <= 20.0 * extent)
                        .ok_or(
                            "円を当てはめられませんでした (円弧や、円周上の線を選んでください)",
                        )?;
                    Fit::Circle(c, r)
                }
            },
            2 => {
                let f = fit_ellipse(&pts)
                    .filter(|(_, rx, ry, _)| rx.max(*ry) <= 20.0 * extent && rx.min(*ry) >= sw);
                let (c, mut rx, mut ry, mut rot) = f.unwrap_or((
                    [(x0 + x1) / 2.0, (y0 + y1) / 2.0],
                    (x1 - x0) / 2.0,
                    (y1 - y0) / 2.0,
                    0.0,
                ));
                // ほぼ水平・垂直の軸は厳密にそろえる
                let q = std::f64::consts::FRAC_PI_2;
                let k = (rot / q).round();
                if (rot - k * q).abs() < 3f64.to_radians() || (rx - ry).abs() < 0.02 * rx {
                    if (k as i64).rem_euclid(2) == 1 && (rx - ry).abs() >= 0.02 * rx {
                        std::mem::swap(&mut rx, &mut ry);
                    }
                    rot = 0.0;
                }
                if rx.min(ry) < sw {
                    return Err("楕円を当てはめられませんでした".into());
                }
                Fit::Ellipse(c, rx, ry, rot)
            }
            _ => self.fit_rect(&sel, &pts)?,
        };

        // 置き換える要素: 選択していたもの + (指定があれば) 内側にあるもの
        let mut remove: BTreeSet<u32> = sel.iter().map(|e| e.id).collect();
        if clear_inside {
            let tol = 1.5 * sw;
            for e in &self.doc.ents {
                if e.layer.bit() & mask == 0 || remove.contains(&e.id) {
                    continue;
                }
                let bb = e.shape.bbox();
                let corners = [
                    [bb[0], bb[1]],
                    [bb[2], bb[1]],
                    [bb[2], bb[3]],
                    [bb[0], bb[3]],
                ];
                if !corners.iter().any(|q| fit.contains(p(*q), tol)) {
                    continue;
                }
                let s = sample(&e.shape.polylines(2.0), 3.0);
                if !s.is_empty() && s.iter().all(|q| fit.contains(*q, tol)) {
                    remove.insert(e.id);
                }
            }
        }
        let cleared = remove.len() - sel.len();

        // 新しい図形と、中に入れる線
        let mut shapes: Vec<Shape> = Vec::new();
        let seg = |a: P, b: P| Shape::Line { a: v(a), b: v(b) };
        let name = match &fit {
            Fit::Circle(c, r) => {
                shapes.push(Shape::Circle {
                    c: v(*c),
                    r: *r as f32,
                });
                let at = |t: f64| [c[0] + r * t.cos(), c[1] + r * t.sin()];
                round_pattern(pattern, &at, &seg, &mut shapes);
                "円"
            }
            Fit::Ellipse(c, rx, ry, rot) => {
                shapes.push(Shape::Ellipse {
                    c: v(*c),
                    rx: *rx as f32,
                    ry: *ry as f32,
                    rot: *rot as f32,
                });
                let at = |t: f64| {
                    let (x, y) = (rx * t.cos(), ry * t.sin());
                    [
                        c[0] + x * rot.cos() - y * rot.sin(),
                        c[1] + x * rot.sin() + y * rot.cos(),
                    ]
                };
                round_pattern(pattern, &at, &seg, &mut shapes);
                "楕円"
            }
            Fit::Rect(c) => {
                for i in 0..4 {
                    shapes.push(seg(c[i], c[(i + 1) % 4]));
                }
                let mid = |i: usize| geom::mul(geom::add(c[i], c[(i + 1) % 4]), 0.5);
                match pattern {
                    1 => shapes.push(seg(c[1], c[3])),
                    2 => {
                        shapes.push(seg(c[0], c[2]));
                        shapes.push(seg(c[1], c[3]));
                    }
                    3 => {
                        shapes.push(seg(mid(0), mid(2)));
                        shapes.push(seg(mid(3), mid(1)));
                    }
                    _ => {}
                }
                "矩形"
            }
        };
        self.checkpoint();
        self.doc.ents.retain(|e| !remove.contains(&e.id));
        self.sel.clear();
        for s in shapes {
            let id = self.doc.push(s, Layer::Line, lt, 0);
            self.sel.insert(id);
        }
        Ok(if cleared > 0 {
            format!("{name}に整え、内側の {cleared} 個を消しました")
        } else {
            format!("{name}に整えました")
        })
    }

    /// 選択した線から矩形を決める。辺の向きは選択した線の向きの (長さで重みづけした) 平均。
    /// 各辺の位置は、その向きの線のうちいちばん外側のものに合わせる (内側のごみや、
    /// 角からのはみ出しには引きずられない)。その向きの線がない側は、選択全体の端に合わせる。
    fn fit_rect(&self, sel: &[&crate::doc::Entity], pts: &[P]) -> Result<Fit, String> {
        let sw = (self.doc.sw as f64).max(1.0);
        let mut segs: Vec<(P, P)> = Vec::new();
        for e in sel {
            if matches!(e.shape, Shape::Line { .. } | Shape::Poly { .. }) {
                for s in e.shape.polylines(3.0) {
                    for w in s.windows(2) {
                        if geom::dist(p(w[0]), p(w[1])) > 1e-6 {
                            segs.push((p(w[0]), p(w[1])));
                        }
                    }
                }
            }
        }
        // 向き (90° 周期) の重みつき平均
        let (mut c4, mut s4) = (0.0, 0.0);
        for (a, b) in &segs {
            let d = geom::sub(*b, *a);
            let (l, th) = (geom::len(d), d[1].atan2(d[0]));
            c4 += l * (4.0 * th).cos();
            s4 += l * (4.0 * th).sin();
        }
        let mut th = if segs.is_empty() {
            0.0
        } else {
            s4.atan2(c4) / 4.0
        };
        if th.abs() < 3f64.to_radians() {
            th = 0.0;
        }
        let axes = [[th.cos(), th.sin()], [-th.sin(), th.cos()]];
        // 軸 k の向きに平行な線が決める、もう一方の軸での辺の位置 (下限, 上限)
        let mut range = [[0.0f64; 2]; 2];
        for k in 0..2 {
            let (along, across) = (axes[k], axes[1 - k]);
            let (mut lo, mut hi) = (f64::MAX, f64::MIN);
            for q in pts {
                let t = geom::dot(*q, across);
                lo = lo.min(t);
                hi = hi.max(t);
            }
            let par: Vec<(f64, f64)> = segs
                .iter()
                .filter(|(a, b)| geom::dot(geom::unit(geom::sub(*b, *a)), along).abs() >= 0.94)
                .map(|(a, b)| {
                    (
                        geom::dot(geom::mul(geom::add(*a, *b), 0.5), across),
                        geom::dist(*a, *b),
                    )
                })
                .collect();
            let lmax = par.iter().map(|s| s.1).fold(0.0, f64::max);
            let main: Vec<(f64, f64)> = par.into_iter().filter(|s| s.1 >= 0.2 * lmax).collect();
            // いちばん外側の線と、それにほぼ重なる線の平均
            let side = |pick: f64| -> f64 {
                let (mut s, mut w) = (0.0, 0.0);
                for (t, l) in main.iter().filter(|m| (m.0 - pick).abs() <= 2.0 * sw) {
                    s += t * l;
                    w += l;
                }
                s / w
            };
            let (mut r0, mut r1) = (lo, hi);
            if !main.is_empty() {
                let mn = main.iter().map(|m| m.0).fold(f64::MAX, f64::min);
                let mx = main.iter().map(|m| m.0).fold(f64::MIN, f64::max);
                if mx - mn >= 0.2 * (hi - lo).max(2.0 * sw) {
                    r0 = side(mn);
                    r1 = side(mx);
                } else if (mn - lo) <= (hi - mx) {
                    // 片側の辺だけが認識されている
                    r0 = side(mn);
                } else {
                    r1 = side(mx);
                }
            }
            range[1 - k] = [r0, r1];
        }
        let [[u0, u1], [v0, v1]] = range;
        if u1 - u0 < 2.0 * sw || v1 - v0 < 2.0 * sw {
            return Err("矩形を決められませんでした (2 辺以上が分かるように選んでください)".into());
        }
        let at = |u: f64, w: f64| geom::add(geom::mul(axes[0], u), geom::mul(axes[1], w));
        Ok(Fit::Rect([at(u0, v0), at(u1, v0), at(u1, v1), at(u0, v1)]))
    }
}

/// 円・楕円の中に入れる線。`at(t)` は媒介変数 t の周上の点。
fn round_pattern(
    pattern: u8,
    at: &dyn Fn(f64) -> P,
    seg: &dyn Fn(P, P) -> Shape,
    out: &mut Vec<Shape>,
) {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    let mut chord = |t: f64| out.push(seg(at(t), at(t + PI)));
    match pattern {
        1 => chord(-FRAC_PI_4),
        2 => {
            chord(FRAC_PI_4);
            chord(-FRAC_PI_4);
        }
        3 => {
            chord(0.0);
            chord(FRAC_PI_2);
        }
        _ => {}
    }
}
