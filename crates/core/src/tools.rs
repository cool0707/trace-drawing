//! 手修正の応用操作: 寸法による補正、2 本の線の幅揃え、はみ出し (ヒゲ) の除去。

use crate::doc::{p, v, Dim, Editor, Layer, Shape, V};
use crate::geom::{self, P};

/// 2 線分の交点の、線分 a 上の位置 (a.0 からの距離)。b 側は `slack` だけ端を延長して判定する。
fn hit_param(a: (P, P), b: (P, P), slack: f64) -> Option<f64> {
    let da = geom::sub(a.1, a.0);
    let db = geom::sub(b.1, b.0);
    let (la, lb) = (geom::len(da), geom::len(db));
    if la < 1e-6 || lb < 1e-6 {
        return None;
    }
    let den = geom::cross(da, db);
    // ほぼ平行な線どうしは交点とみなさない
    if den.abs() < 0.17 * la * lb {
        return None;
    }
    let w = geom::sub(b.0, a.0);
    let t = geom::cross(w, db) / den;
    let u = geom::cross(w, da) / den;
    if t < -1e-9 || t > 1.0 + 1e-9 || u * lb < -slack || u * lb > lb + slack {
        return None;
    }
    Some(t * la)
}

impl Editor {
    /// なぞり操作の開始。終了までの変更は 1 回の Undo で戻せる。
    pub fn begin_group(&mut self) {
        self.grouping = true;
        self.group_saved = false;
        self.group_touched.clear();
    }
    /// 直前の操作と同じ Undo 単位に、続く操作をまとめる (グループ中に呼ぶ)。
    pub fn group_saved_for_merge(&mut self) {
        self.group_saved = true;
    }
    pub fn end_group(&mut self) {
        self.grouping = false;
        self.group_saved = false;
        self.group_touched.clear();
    }

    fn line_ents(&self, mask: u8) -> Vec<(u32, P, P)> {
        self.doc
            .ents
            .iter()
            .filter(|e| e.layer.bit() & mask != 0)
            .filter_map(|e| match e.shape {
                Shape::Line { a, b } => Some((e.id, p(a), p(b))),
                _ => None,
            })
            .collect()
    }

    // ---------------- つなぎペン ----------------

    /// つなぎペンでなぞった経路 `path` (半径 `r`) から、つなぐ対象の線を選び直す。
    ///
    /// ペンが触れた線のうち、次のどちらかを満たすものだけを選ぶ。
    /// - 線全体がなぞった範囲に収まっている
    /// - その線の近くを、線とだいたい同じ向き (約 30° 以内) に、ペンの半径ぶん以上なぞっている
    ///
    /// 向きは経路の場所ごとに見るので、途中でペンを曲げれば、曲げた先の線も選ばれる (角をつなぐ用)。
    /// 横切っているだけの線は選ばれない。選択は毎回この結果で置き換える。
    pub fn select_along_stroke(&mut self, path: &[P], r: f64, mask: u8) {
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
        // 点から経路までの距離
        let to_path = |q: P| -> f64 {
            if path.len() == 1 {
                return geom::dist(q, path[0]);
            }
            path.windows(2)
                .map(|w| geom::dist_seg(q, w[0], w[1]))
                .fold(f64::MAX, f64::min)
        };
        // 経路の各点での「なぞる向き」: 前後それぞれ半径の半分ほど離れた点を結んだ向き。
        // 途中で曲げても、曲げる前と後でそれぞれの向きになる。
        let mut arc = vec![0.0; path.len()];
        for i in 1..path.len() {
            arc[i] = arc[i - 1] + geom::dist(path[i - 1], path[i]);
        }
        let dirs: Vec<Option<P>> = (0..path.len())
            .map(|k| {
                let (mut i, mut j) = (k, k);
                while i > 0 && arc[k] - arc[i] < 0.5 * r {
                    i -= 1;
                }
                while j + 1 < path.len() && arc[j] - arc[k] < 0.5 * r {
                    j += 1;
                }
                let d = geom::sub(path[j], path[i]);
                (geom::len(d) >= 0.25 * r).then(|| geom::unit(d))
            })
            .collect();
        let cos_tol = 30f64.to_radians().cos();
        for (id, a, b) in self.line_ents(mask) {
            if a[0].max(b[0]) < x0
                || a[0].min(b[0]) > x1
                || a[1].max(b[1]) < y0
                || a[1].min(b[1]) > y1
            {
                continue;
            }
            let len = geom::dist(a, b);
            let n = ((len / (r * 0.5).max(0.5)).ceil() as usize).clamp(1, 400);
            let (mut inside, mut touched) = (0, false);
            for k in 0..=n {
                let q = geom::add(a, geom::mul(geom::sub(b, a), k as f64 / n as f64));
                if to_path(q) <= r {
                    inside += 1;
                    touched = true;
                }
            }
            // 経路が線を横切っただけで標本点に当たらなかった場合も「触れた」とみなす
            if !touched {
                touched = path.iter().any(|q| geom::dist_seg(*q, a, b) <= r);
            }
            if !touched {
                continue;
            }
            if inside == n + 1 {
                self.sel.insert(id);
                continue;
            }
            // この線の近くを、線と同じ向きになぞった長さ
            let d = geom::unit(geom::sub(b, a));
            let mut along = 0.0;
            for k in 0..path.len() {
                if geom::dist_seg(path[k], a, b) > 1.5 * r {
                    continue;
                }
                if dirs[k].is_some_and(|u| geom::dot(u, d).abs() >= cos_tol) {
                    let prev = if k > 0 { arc[k] - arc[k - 1] } else { 0.0 };
                    let next = if k + 1 < path.len() {
                        arc[k + 1] - arc[k]
                    } else {
                        0.0
                    };
                    along += 0.5 * (prev + next);
                }
            }
            if along >= r.min(0.5 * len) && along >= 0.5 * r {
                self.sel.insert(id);
            }
        }
    }

    // ---------------- ヒゲの除去 ----------------

    /// なぞった位置の線を、他の線との交点で区切った 1 区間ぶんだけ消す。
    /// 端の区間なら交点まで切り詰め、中間なら 2 本に分かれ、交点がなければ線ごと消える。
    pub fn trim_at(&mut self, q: P, tol: f64, mask: u8) -> bool {
        let Some(id) = self.pick(q, tol, mask) else {
            return false;
        };
        self.trim_line(id, q, mask)
    }

    /// カーソルの移動 q0→q1 が横切った線を、横切った位置の 1 区間ぶんだけ消す (なぞり消し)。
    /// 近くを通っただけの線には触れない。1 回のなぞりで同じ線を重ねて削ることもしない。
    pub fn trim_stroke(&mut self, q0: P, q1: P, mask: u8) -> bool {
        let hits: Vec<(u32, P)> = self
            .line_ents(mask)
            .into_iter()
            .filter(|l| !self.group_touched.contains(&l.0))
            .filter_map(|(id, a, b)| {
                let (d, e) = (geom::sub(b, a), geom::sub(q1, q0));
                let den = geom::cross(d, e);
                if den.abs() < 1e-9 {
                    return None;
                }
                let w = geom::sub(q0, a);
                let t = geom::cross(w, e) / den;
                let u = geom::cross(w, d) / den;
                ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u))
                    .then(|| (id, geom::add(a, geom::mul(d, t))))
            })
            .collect();
        let mut any = false;
        for (id, q) in hits {
            any |= self.trim_line(id, q, mask);
        }
        any
    }

    fn trim_line(&mut self, id: u32, q: P, mask: u8) -> bool {
        if self.group_touched.contains(&id) {
            return false;
        }
        let lines = self.line_ents(mask);
        let Some(&(_, a, b)) = lines.iter().find(|l| l.0 == id) else {
            return false;
        };
        let len = geom::dist(a, b);
        if len < 1e-6 {
            return false;
        }
        let d = geom::unit(geom::sub(b, a));
        let tq = geom::dot(geom::sub(q, a), d).clamp(0.0, len);
        let slack = (self.doc.sw as f64).max(1.0);
        let (mut lo, mut hi) = (0.0f64, len);
        for &(jid, ja, jb) in &lines {
            if jid == id {
                continue;
            }
            if let Some(t) = hit_param((a, b), (ja, jb), slack) {
                if t <= tq {
                    lo = lo.max(t);
                } else {
                    hi = hi.min(t);
                }
            }
        }
        self.checkpoint();
        let idx = self.doc.ents.iter().position(|e| e.id == id).unwrap();
        let (layer, lt, group) = (
            self.doc.ents[idx].layer,
            self.doc.ents[idx].lt,
            self.doc.ents[idx].group,
        );
        let at = |t: f64| v(geom::add(a, geom::mul(d, t)));
        let eps = 0.5;
        let keep_head = lo > eps;
        let keep_tail = hi < len - eps;
        self.sel.remove(&id);
        if self.grouping {
            self.group_touched.push(id);
        }
        match (keep_head, keep_tail) {
            (false, false) => {
                self.doc.ents.remove(idx);
            }
            (true, false) => self.doc.ents[idx].shape = Shape::Line { a: v(a), b: at(lo) },
            (false, true) => self.doc.ents[idx].shape = Shape::Line { a: at(hi), b: v(b) },
            (true, true) => {
                self.doc.ents[idx].shape = Shape::Line { a: v(a), b: at(lo) };
                let nid = self
                    .doc
                    .push(Shape::Line { a: at(hi), b: v(b) }, layer, lt, group);
                if self.grouping {
                    self.group_touched.push(nid);
                }
            }
        }
        true
    }

    /// 交点から少しだけはみ出した線の先 (ヒゲ) を自動で切り詰める。
    /// `max_len` 以下のはみ出しで、先端がどこにもつながっていないものが対象。
    /// `only_selected` が真なら選択中の線だけを処理する。戻り値は切り詰めた箇所の数。
    pub fn trim_whiskers(&mut self, max_len: f64, only_selected: bool) -> usize {
        let mask = Layer::Line.bit();
        let lines = self.line_ents(mask);
        let slack = (self.doc.sw as f64).max(1.0);
        let free_tol = (self.doc.sw as f64 * 1.2).max(1.5);
        let mut edits: Vec<(u32, P, P)> = Vec::new();
        let mut count = 0;
        for &(id, a, b) in &lines {
            if only_selected && !self.sel.contains(&id) {
                continue;
            }
            let len = geom::dist(a, b);
            if len < 1e-6 {
                continue;
            }
            let d = geom::unit(geom::sub(b, a));
            let bb = [
                a[0].min(b[0]) - slack,
                a[1].min(b[1]) - slack,
                a[0].max(b[0]) + slack,
                a[1].max(b[1]) + slack,
            ];
            // 両端それぞれについて、最も近い交点までの距離と、先端が他の線に触れているか
            let (mut near_a, mut near_b) = (f64::MAX, f64::MAX);
            let (mut free_a, mut free_b) = (true, true);
            for &(jid, ja, jb) in &lines {
                if jid == id {
                    continue;
                }
                if ja[0].max(jb[0]) < bb[0]
                    || ja[0].min(jb[0]) > bb[2]
                    || ja[1].max(jb[1]) < bb[1]
                    || ja[1].min(jb[1]) > bb[3]
                {
                    continue;
                }
                if geom::dist_seg(a, ja, jb) <= free_tol {
                    free_a = false;
                }
                if geom::dist_seg(b, ja, jb) <= free_tol {
                    free_b = false;
                }
                if let Some(t) = hit_param((a, b), (ja, jb), slack) {
                    near_a = near_a.min(t);
                    near_b = near_b.min(len - t);
                }
            }
            let cut_a = free_a && near_a > free_tol && near_a <= max_len;
            let cut_b = free_b && near_b > free_tol && near_b <= max_len;
            let (ta, tb) = (
                if cut_a { near_a } else { 0.0 },
                if cut_b { len - near_b } else { len },
            );
            // 目盛りのように短い線が交差しているだけのものは残す
            if (!cut_a && !cut_b) || tb - ta < 3.0 * (ta + (len - tb)) {
                continue;
            }
            count += cut_a as usize + cut_b as usize;
            edits.push((
                id,
                geom::add(a, geom::mul(d, ta)),
                geom::add(a, geom::mul(d, tb)),
            ));
        }
        if edits.is_empty() {
            return 0;
        }
        self.checkpoint();
        for (id, a, b) in edits {
            if let Some(e) = self.doc.ents.iter_mut().find(|e| e.id == id) {
                e.shape = Shape::Line { a: v(a), b: v(b) };
            }
        }
        count
    }

    // ---------------- 幅揃え ----------------

    /// 選択した 2 本の線 (または頂点数が同じ 2 本の折れ線) を平行にし、間隔を `width` にそろえる。
    /// 2 本の中心線は動かさず、両側を均等に寄せる/離す。
    pub fn equalize_width(&mut self, width: f64) -> Result<(), String> {
        let sel: Vec<(u32, Vec<P>, bool)> = self
            .doc
            .ents
            .iter()
            .filter(|e| self.sel.contains(&e.id))
            .filter_map(|e| match &e.shape {
                Shape::Line { a, b } => Some((e.id, vec![p(*a), p(*b)], false)),
                Shape::Poly { pts, closed } => {
                    Some((e.id, pts.iter().map(|q| p(*q)).collect(), *closed))
                }
                _ => None,
            })
            .collect();
        if sel.len() != 2 {
            return Err("線または折れ線をちょうど 2 本選んでください".into());
        }
        let (ida, pa, ca) = sel[0].clone();
        let (idb, mut pb, cb) = sel[1].clone();
        if pa.len() != pb.len() || ca != cb || pa.len() < 2 {
            return Err(
                "2 本の折れ線の頂点数が違います (同じ形の折れ線どうしを選んでください)".into(),
            );
        }
        if width <= 0.0 {
            return Err("幅は正の数で指定してください".into());
        }
        let n = pa.len();
        // 向きが逆なら片方を反転
        if !ca
            && geom::dist(pa[0], pb[0]) + geom::dist(pa[n - 1], pb[n - 1])
                > geom::dist(pa[0], pb[n - 1]) + geom::dist(pa[n - 1], pb[0])
        {
            pb.reverse();
        }
        let nseg = if ca { n } else { n - 1 };
        // 区間ごとの中心線と、a 側・b 側へのオフセット線 (点, 方向)
        let mut la: Vec<(P, P)> = Vec::with_capacity(nseg);
        let mut lb: Vec<(P, P)> = Vec::with_capacity(nseg);
        for i in 0..nseg {
            let j = (i + 1) % n;
            let da = geom::unit(geom::sub(pa[j], pa[i]));
            let mut db = geom::unit(geom::sub(pb[j], pb[i]));
            if geom::dot(da, db) < 0.0 {
                db = geom::mul(db, -1.0);
            }
            if geom::cross(da, db).abs() > 0.35 {
                return Err("2 本がほぼ平行ではありません".into());
            }
            let d = geom::unit(geom::add(da, db));
            let ma = geom::mul(geom::add(pa[i], pa[j]), 0.5);
            let mb = geom::mul(geom::add(pb[i], pb[j]), 0.5);
            let c = geom::mul(geom::add(ma, mb), 0.5);
            let nrm = [-d[1], d[0]];
            let side = if geom::dot(geom::sub(ma, c), nrm) >= 0.0 {
                1.0
            } else {
                -1.0
            };
            la.push((geom::add(c, geom::mul(nrm, side * width / 2.0)), d));
            lb.push((geom::add(c, geom::mul(nrm, -side * width / 2.0)), d));
        }
        let rebuild = |orig: &[P], lines: &[(P, P)], closed: bool| -> Vec<P> {
            let proj =
                |q: P, l: (P, P)| geom::add(l.0, geom::mul(l.1, geom::dot(geom::sub(q, l.0), l.1)));
            (0..orig.len())
                .map(|i| {
                    let prev = if i > 0 {
                        Some(i - 1)
                    } else if closed {
                        Some(lines.len() - 1)
                    } else {
                        None
                    };
                    let next = if i < lines.len() { Some(i) } else { None };
                    match (prev, next) {
                        (Some(a), Some(b)) => {
                            geom::line_intersect(lines[a].0, lines[a].1, lines[b].0, lines[b].1)
                                .unwrap_or_else(|| proj(orig[i], lines[b]))
                        }
                        (None, Some(b)) => proj(orig[i], lines[b]),
                        (Some(a), None) => proj(orig[i], lines[a]),
                        (None, None) => orig[i],
                    }
                })
                .collect()
        };
        let na = rebuild(&pa, &la, ca);
        let nb = rebuild(&pb, &lb, cb);
        self.checkpoint();
        for (id, pts) in [(ida, na), (idb, nb)] {
            let e = self.doc.ents.iter_mut().find(|e| e.id == id).unwrap();
            match &mut e.shape {
                Shape::Line { a, b } => {
                    // 選択順と pb の反転に関わらず、元の端点に近い側へ割り当てる
                    let (q0, q1) = (v(pts[0]), v(pts[1]));
                    if geom::dist(p(*a), pts[0]) <= geom::dist(p(*a), pts[1]) {
                        *a = q0;
                        *b = q1;
                    } else {
                        *a = q1;
                        *b = q0;
                    }
                }
                Shape::Poly { pts: dst, .. } => {
                    let fwd: f64 = dst
                        .iter()
                        .zip(&pts)
                        .map(|(o, q)| geom::dist(p(*o), *q))
                        .sum();
                    let rev: f64 = dst
                        .iter()
                        .zip(pts.iter().rev())
                        .map(|(o, q)| geom::dist(p(*o), *q))
                        .sum();
                    let it: Vec<V> = if fwd <= rev {
                        pts.iter().map(|q| v(*q)).collect()
                    } else {
                        pts.iter().rev().map(|q| v(*q)).collect()
                    };
                    *dst = it;
                }
                _ => {}
            }
        }
        Ok(())
    }

    // ---------------- 塗り → 線 ----------------

    /// 太い帯として塗りになってしまった部分 (ハッチングがつぶれた壁の 2 本線など) を線に変換する。
    ///
    /// 対象は選択中の塗り。何も選択していなければすべての塗り。
    /// - `center` が偽: 帯の両側の辺を線にする (2 本線)。
    /// - `center` が真: 向かい合う 2 辺の真ん中を通る中心線 1 本にする。
    ///
    /// 輪郭の細かい凹凸や帯の端の短い辺は捨て、ほぼ水平・垂直な辺は水平・垂直にそろえ、
    /// 同一直線上の辺は 1 本にまとめる。戻り値は (変換した塗りの数, 作った線の数)。
    pub fn fills_to_lines(&mut self, center: bool) -> (usize, usize) {
        use crate::post::{merge_collinear, snap_hv, Seg};
        let any_sel = self
            .doc
            .ents
            .iter()
            .any(|e| e.layer == Layer::Fill && self.sel.contains(&e.id));
        let targets: Vec<(u32, Vec<P>)> = self
            .doc
            .ents
            .iter()
            .filter(|e| e.layer == Layer::Fill && (!any_sel || self.sel.contains(&e.id)))
            .filter_map(|e| match &e.shape {
                Shape::Poly { pts, closed: true } if pts.len() >= 3 => {
                    Some((e.id, pts.iter().map(|q| p(*q)).collect()))
                }
                _ => None,
            })
            .collect();
        if targets.is_empty() {
            return (0, 0);
        }
        let sw = (self.doc.sw as f64).max(1.0);
        let mut segs: Vec<Seg> = Vec::new();
        let mut widths: Vec<f64> = Vec::new();
        for (_, poly) in &targets {
            let n = poly.len();
            // 帯の太さの目安: 面積 / (周長の半分)
            let (mut area, mut peri) = (0.0, 0.0);
            for i in 0..n {
                let (a, b) = (poly[i], poly[(i + 1) % n]);
                area += geom::cross(a, b);
                peri += geom::dist(a, b);
            }
            let width = (area.abs() / peri.max(1e-6)).max(sw);
            widths.push(width);
            // 凹凸をならした輪郭にしてから、長い辺だけを取り出す
            let far = (0..n)
                .max_by(|&i, &j| {
                    geom::dist(poly[i], poly[0])
                        .partial_cmp(&geom::dist(poly[j], poly[0]))
                        .unwrap()
                })
                .unwrap();
            let mut ring: Vec<P> = (0..n).map(|k| poly[(far + k) % n]).collect();
            ring.push(ring[0]);
            let keep = geom::douglas_peucker(&ring, (2.0 * sw).max(0.3 * width));
            let min_len = (4.0 * sw).max(1.5 * width);
            for w in keep.windows(2) {
                let (a, b) = (ring[w[0]], ring[w[1]]);
                if geom::dist(a, b) < min_len {
                    continue;
                }
                // 元の頂点で当てはめ直す
                let (c, d) = geom::fit_line(&ring[w[0]..=w[1]], None);
                let pa = geom::add(c, geom::mul(d, geom::dot(geom::sub(a, c), d)));
                let pb = geom::add(c, geom::mul(d, geom::dot(geom::sub(b, c), d)));
                segs.push(Seg {
                    a: pa,
                    b: pb,
                    w: width,
                    small: None,
                    lt: crate::doc::LineType::Solid,
                    hatch: 0,
                    alive: true,
                });
            }
        }
        snap_hv(&mut segs, 4.0);
        let mut consumed = vec![false; 1];
        merge_collinear(
            &mut segs,
            3.0 * sw,
            3.0 * sw,
            1.5 * sw,
            &mut consumed,
            &|_, _| 0.0,
        );
        let mut out: Vec<(P, P, f64)> = Vec::new();
        // 向かい合う平行な辺 (帯の反対側) を探す。相手のいない短い辺は、不定形なかたまりの輪郭なので捨てる。
        let live: Vec<usize> = (0..segs.len()).filter(|&i| segs[i].alive).collect();
        let wmax = widths.iter().cloned().fold(0.0, f64::max) * 3.0;
        let partner = |i: usize| -> Option<(usize, f64)> {
            let (a, d, li) = (segs[i].a, segs[i].dir(), segs[i].len());
            let mut best: Option<(usize, f64)> = None;
            for &j in &live {
                if j == i || geom::cross(d, segs[j].dir()).abs() > 0.12 {
                    continue;
                }
                let mid = geom::mul(geom::add(segs[j].a, segs[j].b), 0.5);
                let off = geom::line_offset(mid, a, d).abs();
                if off < 1.5 * sw || off > wmax.min(3.0 * segs[i].w.max(segs[j].w)) {
                    continue;
                }
                let (t0, t1) = (
                    geom::dot(geom::sub(segs[j].a, a), d),
                    geom::dot(geom::sub(segs[j].b, a), d),
                );
                let ov = t0.max(t1).min(li) - t0.min(t1).max(0.0);
                if ov < 0.5 * li.min(segs[j].len()) {
                    continue;
                }
                if best.is_none_or(|b| off < b.1) {
                    best = Some((j, off));
                }
            }
            best
        };
        let partners: Vec<Option<(usize, f64)>> = (0..segs.len())
            .map(|i| if segs[i].alive { partner(i) } else { None })
            .collect();
        if center {
            // 組になった 2 辺の真ん中を通る線にする
            let mut used = vec![false; segs.len()];
            for &i in &live {
                let Some((j, off)) = partners[i] else {
                    continue;
                };
                if used[i] || used[j] || partners[j].map(|x| x.0) != Some(i) {
                    continue;
                }
                used[i] = true;
                used[j] = true;
                // 中心線: 2 辺の平均の向きで、両方の端を覆う範囲
                let mut dj = segs[j].dir();
                let di = segs[i].dir();
                if geom::dot(di, dj) < 0.0 {
                    dj = geom::mul(dj, -1.0);
                }
                let d = geom::unit(geom::add(di, dj));
                let c = geom::mul(
                    geom::add(
                        geom::add(segs[i].a, segs[i].b),
                        geom::add(segs[j].a, segs[j].b),
                    ),
                    0.25,
                );
                let ts: Vec<f64> = [segs[i].a, segs[i].b, segs[j].a, segs[j].b]
                    .iter()
                    .map(|q| geom::dot(geom::sub(*q, c), d))
                    .collect();
                let (t0, t1) = (
                    ts.iter().cloned().fold(f64::MAX, f64::min),
                    ts.iter().cloned().fold(f64::MIN, f64::max),
                );
                out.push((
                    geom::add(c, geom::mul(d, t0)),
                    geom::add(c, geom::mul(d, t1)),
                    off,
                ));
            }
            // 相手のいない長い辺はそのまま線として残す
            for &i in &live {
                if !used[i] && segs[i].len() >= 4.0 * segs[i].w {
                    out.push((segs[i].a, segs[i].b, 0.0));
                }
            }
        } else {
            for &i in &live {
                if partners[i].is_some() || segs[i].len() >= 4.0 * segs[i].w {
                    out.push((segs[i].a, segs[i].b, 0.0));
                }
            }
        }
        self.checkpoint();
        let ids: std::collections::BTreeSet<u32> = targets.iter().map(|t| t.0).collect();
        self.doc.ents.retain(|e| !ids.contains(&e.id));
        self.sel.clear();
        for (a, b, w) in &out {
            let id = self.doc.push(
                Shape::Line { a: v(*a), b: v(*b) },
                Layer::Line,
                crate::doc::LineType::Solid,
                0,
            );
            if let Some(e) = self.doc.ents.last_mut() {
                e.w = *w as f32;
            }
            self.sel.insert(id);
        }
        (ids.len(), out.len())
    }

    // ---------------- 方眼の除去 ----------------

    /// 方眼紙の線をまとめて消す。
    ///
    /// 選択中の水平・垂直の線 (方眼の線を縦横それぞれ数本) から方眼の間隔と位置を求め、
    /// その格子に乗っている水平・垂直の線をすべて削除する。
    /// - 間隔は同じ向きの線を 2 本以上選んだ側から求める。片方の向きが 1 本だけなら、もう一方と同じ間隔 (正方形の方眼) とみなす。
    /// - 選んだ線より明らかに太い線は、方眼の上に描かれた図の線とみなして残す。
    ///
    /// 戻り値は削除した本数。
    pub fn remove_grid(&mut self, mask: u8) -> Result<usize, String> {
        // (id, 向き 0=水平/1=垂直, 位置, 太さ)
        let axis_lines: Vec<(u32, usize, f64, f64)> = self
            .doc
            .ents
            .iter()
            .filter(|e| e.layer.bit() & mask != 0)
            .filter_map(|e| match e.shape {
                Shape::Line { a, b } => {
                    let (dx, dy) = ((b[0] - a[0]).abs() as f64, (b[1] - a[1]).abs() as f64);
                    let len = dx.hypot(dy);
                    if len < 1e-6 {
                        None
                    } else if dy <= 0.035 * len {
                        Some((e.id, 0, (a[1] + b[1]) as f64 / 2.0, e.w as f64))
                    } else if dx <= 0.035 * len {
                        Some((e.id, 1, (a[0] + b[0]) as f64 / 2.0, e.w as f64))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect();
        let mut picked: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
        let mut wmax: f64 = 0.0;
        for l in axis_lines.iter().filter(|l| self.sel.contains(&l.0)) {
            picked[l.1].push(l.2);
            wmax = wmax.max(l.3);
        }
        if picked[0].len() + picked[1].len() < 2 {
            return Err(
                "方眼の線を、同じ向きに 2 本以上 (できれば縦横それぞれ数本) 選んでください".into(),
            );
        }
        // 向きごとの間隔: 選んだ線どうしの最小の間隔を基準に、整数倍として当てはめ直す
        let pitch_of = |pos: &[f64]| -> Option<f64> {
            let mut s = pos.to_vec();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            s.dedup_by(|a, b| (*a - *b).abs() < 1.0);
            if s.len() < 2 {
                return None;
            }
            let p0 = s.windows(2).map(|w| w[1] - w[0]).fold(f64::MAX, f64::min);
            // 最小二乗: pos = o + k p
            let ks: Vec<f64> = s.iter().map(|x| ((x - s[0]) / p0).round()).collect();
            let n = s.len() as f64;
            let (mk, mx) = (ks.iter().sum::<f64>() / n, s.iter().sum::<f64>() / n);
            let num: f64 = ks.iter().zip(&s).map(|(k, x)| (k - mk) * (x - mx)).sum();
            let den: f64 = ks.iter().map(|k| (k - mk) * (k - mk)).sum();
            (den > 0.0).then(|| num / den)
        };
        let p = [pitch_of(&picked[0]), pitch_of(&picked[1])];
        let Some(fallback) = p[0].or(p[1]) else {
            return Err("方眼の間隔が分かりません。同じ向きの線を 2 本以上選んでください".into());
        };
        let sw = (self.doc.sw as f64).max(1.0);
        if fallback < 3.0 * sw {
            return Err("選んだ線どうしが近すぎて方眼の間隔を決められません".into());
        }
        // 太さの上限: 選んだ線の太さと、格子に乗っている線の大多数の太さの両方から決める
        // (同じ方眼線でもスキャンのにじみで太さが 2〜3 倍ばらつく)
        let mut on_grid_w: Vec<f64> = Vec::new();
        for dir in 0..2 {
            if picked[dir].is_empty() {
                continue;
            }
            let (pitch, origin) = (p[dir].unwrap_or(fallback), picked[dir][0]);
            let tol = (1.5 * sw).max(0.06 * pitch).min(0.25 * pitch);
            for l in axis_lines.iter().filter(|l| l.1 == dir && l.3 > 0.0) {
                let k = ((l.2 - origin) / pitch).round();
                if (l.2 - origin - k * pitch).abs() <= tol {
                    on_grid_w.push(l.3);
                }
            }
        }
        on_grid_w.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let typical = on_grid_w
            .get(on_grid_w.len() * 3 / 4)
            .copied()
            .unwrap_or(0.0);
        let wlimit = if wmax > 0.0 || typical > 0.0 {
            (1.4 * wmax + 0.5).max(1.8 * typical + 0.5)
        } else {
            f64::MAX
        };
        let mut remove: Vec<u32> = Vec::new();
        for dir in 0..2 {
            if picked[dir].is_empty() {
                continue;
            }
            let mut pitch = p[dir].unwrap_or(fallback);
            let mut origin = picked[dir][0];
            let tol = (1.5 * sw).max(0.06 * pitch).min(0.25 * pitch);
            let on_grid = |x: f64, origin: f64, pitch: f64| -> Option<f64> {
                let k = ((x - origin) / pitch).round();
                ((x - origin - k * pitch).abs() <= tol).then_some(k)
            };
            // 格子に乗った線すべてで間隔と位置を当てはめ直す (紙の伸びで少しずつずれるため)
            for _ in 0..3 {
                let hits: Vec<(f64, f64)> = axis_lines
                    .iter()
                    .filter(|l| l.1 == dir && l.3 <= wlimit)
                    .filter_map(|l| on_grid(l.2, origin, pitch).map(|k| (k, l.2)))
                    .collect();
                if hits.len() < 2 {
                    break;
                }
                let n = hits.len() as f64;
                let (mk, mx) = (
                    hits.iter().map(|h| h.0).sum::<f64>() / n,
                    hits.iter().map(|h| h.1).sum::<f64>() / n,
                );
                let den: f64 = hits.iter().map(|h| (h.0 - mk) * (h.0 - mk)).sum();
                if den <= 0.0 {
                    break;
                }
                let np = hits.iter().map(|h| (h.0 - mk) * (h.1 - mx)).sum::<f64>() / den;
                if (np - pitch).abs() > 0.05 * pitch {
                    break;
                }
                pitch = np;
                origin = mx - mk * np;
            }
            for l in axis_lines.iter().filter(|l| l.1 == dir) {
                let selected = self.sel.contains(&l.0);
                if selected || (l.3 <= wlimit && on_grid(l.2, origin, pitch).is_some()) {
                    remove.push(l.0);
                }
            }
        }
        if remove.is_empty() {
            return Ok(0);
        }
        self.checkpoint();
        let set: std::collections::BTreeSet<u32> = remove.into_iter().collect();
        self.doc.ents.retain(|e| !set.contains(&e.id));
        self.sel.clear();
        Ok(set.len())
    }

    // ---------------- 寸法による補正 ----------------

    /// 2 点間の実寸 [mm] を書き入れる。
    pub fn add_dim(&mut self, a: P, b: P, len_mm: f64) -> u32 {
        self.checkpoint();
        self.doc.next_id += 1;
        let id = self.doc.next_id;
        self.doc.dims.push(Dim {
            id,
            a: v(a),
            b: v(b),
            len: len_mm as f32,
        });
        id
    }

    pub fn pick_dim(&self, q: P, tol: f64) -> Option<u32> {
        self.doc
            .dims
            .iter()
            .map(|d| (geom::dist_seg(q, p(d.a), p(d.b)), d.id))
            .filter(|(d, _)| *d <= tol)
            .min_by(|x, y| x.0.partial_cmp(&y.0).unwrap())
            .map(|x| x.1)
    }

    pub fn remove_dim(&mut self, id: u32) -> bool {
        if !self.doc.dims.iter().any(|d| d.id == id) {
            return false;
        }
        self.checkpoint();
        self.doc.dims.retain(|d| d.id != id);
        true
    }

    pub fn set_dim(&mut self, id: u32, len_mm: f64) -> bool {
        if !self.doc.dims.iter().any(|d| d.id == id) {
            return false;
        }
        self.checkpoint();
        for d in self.doc.dims.iter_mut().filter(|d| d.id == id) {
            d.len = len_mm as f32;
        }
        true
    }

    /// 書き入れた寸法を正として図面全体を補正する。
    ///
    /// - 寸法 1 本: 縮尺のみ
    /// - 向きの違う寸法 2 本: 縦横それぞれの縮尺 (スキャナの送り方向の伸び縮み)
    /// - 3 本以上: 縦横の縮尺 + せん断 (平行四辺形の歪み)
    ///
    /// 戻り値は結果の説明。補正後は図面座標 1 単位の実寸が `doc.calib.unit` [mm] に入る。
    pub fn calibrate(&mut self) -> Result<String, String> {
        let dims: Vec<(P, f64)> = self
            .doc
            .dims
            .iter()
            .map(|d| (geom::sub(p(d.b), p(d.a)), d.len as f64))
            .filter(|(d, l)| geom::len(*d) > 1.0 && *l > 0.0)
            .collect();
        if dims.is_empty() {
            return Err(
                "先に「寸法」ツールで、長さの分かっている箇所に寸法を書き入れてください".into(),
            );
        }
        // 未知数は計量行列 M = [[m0, m1], [m1, m2]] で、各寸法が d^T M d = L^2 を与える (線形)。
        // 長さで割って正規化し、相対誤差を最小にする。
        let rows: Vec<([f64; 3], f64)> = dims
            .iter()
            .map(|(d, l)| {
                let u = geom::unit(*d);
                (
                    [u[0] * u[0], 2.0 * u[0] * u[1], u[1] * u[1]],
                    (l / geom::len(*d)).powi(2),
                )
            })
            .collect();
        // 向きのばらつき: 水平寄り・垂直寄りの寸法がどれだけあるか
        let sx: f64 = rows.iter().map(|r| r.0[0]).sum();
        let sy: f64 = rows.iter().map(|r| r.0[2]).sum();
        let n = rows.len() as f64;
        let solve = |cols: &[usize]| -> Option<[f64; 3]> {
            let k = cols.len();
            let mut a = [[0.0f64; 4]; 3];
            for (r, y) in &rows {
                for i in 0..k {
                    for j in 0..k {
                        a[i][j] += r[cols[i]] * r[cols[j]];
                    }
                    a[i][3] += r[cols[i]] * y;
                }
            }
            for c in 0..k {
                let piv =
                    (c..k).max_by(|&x, &y| a[x][c].abs().partial_cmp(&a[y][c].abs()).unwrap())?;
                if a[piv][c].abs() < 1e-6 {
                    return None;
                }
                a.swap(c, piv);
                for r in 0..k {
                    if r != c {
                        let f = a[r][c] / a[c][c];
                        for q in c..4 {
                            a[r][q] -= f * a[c][q];
                        }
                    }
                }
            }
            let mut out = [0.0; 3];
            for i in 0..k {
                out[cols[i]] = a[i][3] / a[i][i];
            }
            Some(out)
        };
        let uniform = rows.iter().map(|r| r.1).sum::<f64>() / n;
        let both = sx > 0.15 * n && sy > 0.15 * n;
        let (m, kind) = if rows.len() >= 3 && both {
            match solve(&[0, 1, 2]) {
                Some(m)
                    if m[0] > 0.0
                        && m[2] > 0.0
                        && m[0] * m[2] - m[1] * m[1] > 0.05 * m[0] * m[2] =>
                {
                    (m, "縦横の縮尺とせん断")
                }
                _ => match solve(&[0, 2]) {
                    Some(m) if m[0] > 0.0 && m[2] > 0.0 => (m, "縦横の縮尺"),
                    _ => ([uniform, 0.0, uniform], "縮尺"),
                },
            }
        } else if rows.len() >= 2 && both {
            match solve(&[0, 2]) {
                Some(m) if m[0] > 0.0 && m[2] > 0.0 => (m, "縦横の縮尺"),
                _ => ([uniform, 0.0, uniform], "縮尺"),
            }
        } else {
            ([uniform, 0.0, uniform], "縮尺")
        };
        // 縦横比が極端に変わる解は入力ミスとみなす
        let ratio = (m[0] / m[2]).sqrt();
        if !(0.5..=2.0).contains(&ratio) {
            return Err("寸法どうしが大きく矛盾しています。入力した長さを確認してください".into());
        }
        // A = sqrt(M) (対称)。固有分解して平方根を取る。
        let th = 0.5 * (2.0 * m[1]).atan2(m[0] - m[2]);
        let (c, s) = (th.cos(), th.sin());
        let e1 = m[0] * c * c + 2.0 * m[1] * c * s + m[2] * s * s;
        let e2 = m[0] * s * s - 2.0 * m[1] * c * s + m[2] * c * c;
        let (r1, r2) = (e1.max(1e-12).sqrt(), e2.max(1e-12).sqrt());
        let a = [
            r1 * c * c + r2 * s * s,
            (r1 - r2) * c * s,
            (r1 - r2) * c * s,
            r1 * s * s + r2 * c * c,
        ];
        let unit = (a[0] * a[3] - a[1] * a[2]).sqrt(); // mm / 単位
        let k = [a[0] / unit, a[1] / unit, a[2] / unit, a[3] / unit]; // 面積を変えない形の補正

        self.checkpoint();
        let f = |q: V| -> V {
            [
                (k[0] * q[0] as f64 + k[1] * q[1] as f64) as f32,
                (k[2] * q[0] as f64 + k[3] * q[1] as f64) as f32,
            ]
        };
        for e in self.doc.ents.iter_mut() {
            match &mut e.shape {
                Shape::Line { a, b } => {
                    *a = f(*a);
                    *b = f(*b);
                }
                Shape::Poly { pts, .. } => pts.iter_mut().for_each(|q| *q = f(*q)),
                Shape::Circle { c, .. } | Shape::Arc { c, .. } | Shape::Ellipse { c, .. } => {
                    *c = f(*c)
                }
                Shape::Text { bbox, strokes, .. } => {
                    let (p0, p1) = (f([bbox[0], bbox[1]]), f([bbox[2], bbox[3]]));
                    *bbox = [
                        p0[0].min(p1[0]),
                        p0[1].min(p1[1]),
                        p0[0].max(p1[0]),
                        p0[1].max(p1[1]),
                    ];
                    strokes
                        .iter_mut()
                        .for_each(|st| st.iter_mut().for_each(|q| *q = f(*q)));
                }
            }
        }
        for d in self.doc.dims.iter_mut() {
            d.a = f(d.a);
            d.b = f(d.b);
        }
        let corner = f([self.doc.w, self.doc.h]);
        self.doc.w = corner[0].max(f([self.doc.w, 0.0])[0]);
        self.doc.h = corner[1].max(f([0.0, self.doc.h])[1]);
        let pm = self.doc.calib.m.map(|x| x as f64);
        self.doc.calib.m = [
            (k[0] * pm[0] + k[1] * pm[2]) as f32,
            (k[0] * pm[1] + k[1] * pm[3]) as f32,
            (k[2] * pm[0] + k[3] * pm[2]) as f32,
            (k[2] * pm[1] + k[3] * pm[3]) as f32,
        ];
        self.doc.calib.unit = unit as f32;
        // 補正後の残差
        let worst = self
            .doc
            .dims
            .iter()
            .map(|d| ((geom::dist(p(d.a), p(d.b)) * unit - d.len as f64) / d.len as f64).abs())
            .fold(0.0, f64::max);
        Ok(format!(
            "{kind}を補正しました（1 単位 = {:.4} mm、縦横比の補正 {:+.2}%、寸法との最大誤差 {:.2}%）",
            unit,
            (ratio - 1.0) * 100.0,
            worst * 100.0
        ))
    }
}
