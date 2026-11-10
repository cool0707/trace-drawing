//! 線分の後処理: 水平垂直スナップ、同一直線の統合、破線の一本化、
//! ハッチング検出、端点の突き合わせ、文字のかたまりの分離。

use crate::doc::LineType;
use crate::geom::*;
use crate::morph::Comp;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Seg {
    pub a: P,
    pub b: P,
    pub w: f64,
    /// 単独の短い直線だけからなる小さな連結成分 (破線の 1 片や文字の横棒の候補) ならその番号
    pub small: Option<u32>,
    pub lt: LineType,
    pub hatch: u32,
    pub alive: bool,
}

impl Seg {
    pub fn len(&self) -> f64 {
        dist(self.a, self.b)
    }
    pub fn dir(&self) -> P {
        unit(sub(self.b, self.a))
    }
}

struct Grid {
    cell: f64,
    map: HashMap<(i32, i32), Vec<u32>>,
}

impl Grid {
    fn new(cell: f64) -> Self {
        Grid {
            cell: cell.max(1.0),
            map: HashMap::new(),
        }
    }
    fn key(&self, p: P) -> (i32, i32) {
        (
            (p[0] / self.cell).floor() as i32,
            (p[1] / self.cell).floor() as i32,
        )
    }
    fn insert(&mut self, p: P, id: u32) {
        let k = self.key(p);
        let v = self.map.entry(k).or_default();
        if v.last() != Some(&id) {
            v.push(id);
        }
    }
    fn insert_seg(&mut self, a: P, b: P, id: u32) {
        let n = (dist(a, b) / (self.cell * 0.5)).ceil().max(1.0) as usize;
        for i in 0..=n {
            let t = i as f64 / n as f64;
            self.insert(add(a, mul(sub(b, a), t)), id);
        }
    }
    fn near(&self, p: P, out: &mut Vec<u32>) {
        out.clear();
        let (kx, ky) = self.key(p);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(v) = self.map.get(&(kx + dx, ky + dy)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

/// ほぼ水平・垂直な線を厳密な水平・垂直にする。
pub fn snap_hv(segs: &mut [Seg], tol_deg: f64) {
    let t = tol_deg.to_radians().tan();
    if t <= 0.0 {
        return;
    }
    for s in segs.iter_mut() {
        let (dx, dy) = ((s.b[0] - s.a[0]).abs(), (s.b[1] - s.a[1]).abs());
        if dy <= t * dx {
            let m = (s.a[1] + s.b[1]) / 2.0;
            s.a[1] = m;
            s.b[1] = m;
        } else if dx <= t * dy {
            let m = (s.a[0] + s.b[0]) / 2.0;
            s.a[0] = m;
            s.b[0] = m;
        }
    }
}

/// `s` が `l` の載る直線上にあれば、直線方向のすき間 (重なっていれば 0) を返す。
fn collinear_gap(l: &Seg, s: &Seg, tol: f64) -> Option<f64> {
    let d = l.dir();
    if line_offset(s.a, l.a, d).abs() > tol || line_offset(s.b, l.a, d).abs() > tol {
        return None;
    }
    if s.len() > 2.5 * tol && cross(d, s.dir()).abs() > 0.3 {
        return None;
    }
    let ll = l.len();
    let t0 = dot(sub(s.a, l.a), d);
    let t1 = dot(sub(s.b, l.a), d);
    let (lo, hi) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
    Some(if lo > ll {
        lo - ll
    } else if hi < 0.0 {
        -hi
    } else {
        0.0
    })
}

/// 複数の線分を 1 本の線分にまとめる (長さ重み付きの直線当てはめ)。
fn merged_line(parts: &[&Seg]) -> (P, P) {
    let mut pts = Vec::with_capacity(parts.len() * 2);
    let mut wts = Vec::with_capacity(parts.len() * 2);
    for s in parts {
        let w = s.len().max(0.5);
        pts.push(s.a);
        pts.push(s.b);
        wts.push(w);
        wts.push(w);
    }
    let (c, d) = fit_line(&pts, Some(&wts));
    let (mut t0, mut t1) = (f64::MAX, f64::MIN);
    for p in &pts {
        let t = dot(sub(*p, c), d);
        t0 = t0.min(t);
        t1 = t1.max(t);
    }
    (add(c, mul(d, t0)), add(c, mul(d, t1)))
}

fn order<'a>(x: &'a Seg, y: &'a Seg) -> (&'a Seg, &'a Seg) {
    if x.len() >= y.len() {
        (x, y)
    } else {
        (y, x)
    }
}

/// 同一直線上で接している (すき間が `gap` 以下の) 線分を 1 本にまとめる。
/// 交差で分断された線や、かすれで切れた線がつながる。
///
/// `ink(p, q)` は p-q 間がインクで埋まっている割合。すき間が広くてもインクでつながっていれば
/// (他の線との交差部など) 実線として統合し、白く空いていれば破線の候補として残す。
pub fn merge_collinear(
    segs: &mut [Seg],
    gap: f64,
    far_gap: f64,
    tol: f64,
    consumed: &mut [bool],
    ink: &dyn Fn(P, P) -> f64,
) {
    let mut cand = Vec::new();
    for _pass in 0..12 {
        let mut grid = Grid::new(far_gap.max(gap).max(8.0));
        for (i, s) in segs.iter().enumerate() {
            if s.alive {
                grid.insert(s.a, i as u32);
                grid.insert(s.b, i as u32);
            }
        }
        let mut any = false;
        for i in 0..segs.len() {
            if !segs[i].alive {
                continue;
            }
            for end in 0..2 {
                let e = if end == 0 { segs[i].a } else { segs[i].b };
                grid.near(e, &mut cand);
                for &j in &cand {
                    let j = j as usize;
                    if j == i || !segs[j].alive {
                        continue;
                    }
                    let (l, s) = order(&segs[i], &segs[j]);
                    let Some(g) = collinear_gap(l, s, tol) else {
                        continue;
                    };
                    if g > gap {
                        if g > far_gap {
                            continue;
                        }
                        // 向かい合う端点の間がインクで埋まっているか
                        let mut best = (f64::MAX, segs[i].a, segs[j].a);
                        for p in [segs[i].a, segs[i].b] {
                            for q in [segs[j].a, segs[j].b] {
                                if dist(p, q) < best.0 {
                                    best = (dist(p, q), p, q);
                                }
                            }
                        }
                        if ink(best.1, best.2) < 0.8 {
                            continue;
                        }
                    }
                    let (a, b) = merged_line(&[&segs[i], &segs[j]]);
                    let (li, lj) = (segs[i].len(), segs[j].len());
                    for k in [i, j] {
                        if let Some(c) = segs[k].small {
                            consumed[c as usize] = true;
                        }
                    }
                    segs[i].w = (segs[i].w * li + segs[j].w * lj) / (li + lj).max(1e-6);
                    segs[i].a = a;
                    segs[i].b = b;
                    segs[i].small = None;
                    segs[j].alive = false;
                    any = true;
                }
            }
        }
        if !any {
            break;
        }
    }
}

pub struct DashOpts {
    pub max_gap: f64,
    pub tol: f64,
    pub min_parts: usize,
    pub sw: f64,
}

/// 等間隔に並んだ同一直線上の短い線の列を、破線 (または一点鎖線) 1 本にまとめる。
pub fn merge_dashed(segs: &mut [Seg], o: &DashOpts, consumed: &mut [bool]) {
    let n = segs.len();
    let mut grid = Grid::new(o.max_gap);
    for (i, s) in segs.iter().enumerate() {
        if s.alive {
            grid.insert(s.a, (i * 2) as u32);
            grid.insert(s.b, (i * 2 + 1) as u32);
        }
    }
    let end_of = |segs: &[Seg], e: usize| -> P {
        if e % 2 == 0 {
            segs[e / 2].a
        } else {
            segs[e / 2].b
        }
    };
    // 各端点から見た最良の相手端点
    let mut link: Vec<Option<(usize, f64)>> = vec![None; n * 2];
    let mut cand = Vec::new();
    for e in 0..n * 2 {
        let i = e / 2;
        if !segs[i].alive {
            continue;
        }
        let pe = end_of(segs, e);
        let po = end_of(segs, e ^ 1);
        grid.near(pe, &mut cand);
        let mut best: Option<(usize, f64)> = None;
        for &f in &cand {
            let f = f as usize;
            let j = f / 2;
            if j == i || !segs[j].alive {
                continue;
            }
            let pf = end_of(segs, f);
            let pfo = end_of(segs, f ^ 1);
            let g = dist(pe, pf);
            if g > o.max_gap || g < 0.5 {
                continue;
            }
            // 向かい合った端どうしであること
            if dist(po, pf) < g || dist(pe, pfo) < g {
                continue;
            }
            let (l, s) = order(&segs[i], &segs[j]);
            let Some(ag) = collinear_gap(l, s, o.tol) else {
                continue;
            };
            if ag <= 0.0 || g > 1.5 * l.len() + 2.0 * o.sw {
                continue;
            }
            if best.is_none_or(|b| g < b.1) {
                best = Some((f, g));
            }
        }
        link[e] = best;
    }
    // 相互に最良のリンクだけ残す
    let mutual: Vec<Option<(usize, f64)>> = (0..n * 2)
        .map(|e| link[e].filter(|(f, _)| link[*f].is_some_and(|(b, _)| b == e)))
        .collect();
    let mut done = vec![false; n];
    for start in 0..n {
        if !segs[start].alive || done[start] {
            continue;
        }
        let (m0, m1) = (mutual[start * 2], mutual[start * 2 + 1]);
        if m0.is_some() == m1.is_some() {
            continue; // 列の端 (片側だけリンク) から始める
        }
        let mut chain = vec![start];
        let mut gaps = Vec::new();
        let mut e = if m0.is_some() {
            start * 2
        } else {
            start * 2 + 1
        };
        done[start] = true;
        while let Some((f, g)) = mutual[e] {
            let j = f / 2;
            if done[j] {
                break;
            }
            done[j] = true;
            chain.push(j);
            gaps.push(g);
            e = f ^ 1;
        }
        if chain.len() < o.min_parts {
            continue;
        }
        // すき間が不ぞろいな所で列を切り、それぞれを破線候補にする
        let mut sg = gaps.clone();
        sg.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med_all = sg[sg.len() / 2];
        let mut from = 0;
        for k in 0..=gaps.len() {
            if k == gaps.len() || gaps[k] > 2.5 * med_all + o.sw {
                if k + 1 - from >= o.min_parts {
                    finish_dash(segs, &chain[from..=k], &gaps[from..k], o, consumed);
                }
                from = k + 1;
            }
        }
    }
    merge_broken_dashed(segs, o, consumed);
}

fn finish_dash(
    segs: &mut [Seg],
    chain: &[usize],
    gaps: &[f64],
    o: &DashOpts,
    consumed: &mut [bool],
) {
    let mut sg = gaps.to_vec();
    sg.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med_gap = sg[sg.len() / 2];
    let parts: Vec<&Seg> = chain.iter().map(|&i| &segs[i]).collect();
    let (a, b) = merged_line(&parts);
    let d = unit(sub(b, a));
    if parts.iter().any(|s| {
        line_offset(s.a, a, d).abs() > 2.0 * o.tol || line_offset(s.b, a, d).abs() > 2.0 * o.tol
    }) {
        return;
    }
    let mut lens: Vec<f64> = parts.iter().map(|s| s.len()).collect();
    lens.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let max_len = lens[lens.len() - 1];
    let tiny = lens.iter().filter(|l| **l < 0.25 * max_len).count();
    // 長い線の列は一点鎖線 (点はかすれて消えていることが多い)、短い線の列は破線
    let lt = if lens[lens.len() / 2] >= 6.0 * med_gap
        || lens[lens.len() / 2] >= 12.0 * o.sw
        || (chain.len() >= 4 && tiny * 3 >= chain.len() && tiny * 3 <= chain.len() * 2)
    {
        LineType::Chain
    } else {
        LineType::Dashed
    };
    let w = parts.iter().map(|s| s.w).sum::<f64>() / parts.len() as f64;
    for &i in chain {
        if let Some(c) = segs[i].small {
            consumed[c as usize] = true;
        }
        segs[i].alive = false;
    }
    let s0 = &mut segs[chain[0]];
    s0.alive = true;
    s0.a = a;
    s0.b = b;
    s0.w = w;
    s0.lt = lt;
    s0.small = None;
}

/// 他の線や塗りに遮られて切れた破線どうし、および破線の延長上に残った切れ端をつなぐ。
fn merge_broken_dashed(segs: &mut [Seg], o: &DashOpts, consumed: &mut [bool]) {
    let reach = 3.0 * o.max_gap;
    let mut cand = Vec::new();
    for _pass in 0..8 {
        let mut grid = Grid::new(reach);
        for (i, s) in segs.iter().enumerate() {
            if s.alive {
                grid.insert(s.a, i as u32);
                grid.insert(s.b, i as u32);
            }
        }
        let mut any = false;
        for i in 0..segs.len() {
            if !segs[i].alive || segs[i].lt == LineType::Solid {
                continue;
            }
            for end in 0..2 {
                let e = if end == 0 { segs[i].a } else { segs[i].b };
                grid.near(e, &mut cand);
                for &j in &cand {
                    let j = j as usize;
                    if j == i || !segs[j].alive {
                        continue;
                    }
                    let solid = segs[j].lt == LineType::Solid;
                    // 実線の切れ端は短いものだけ取り込む
                    if solid && segs[j].len() > 2.0 * o.max_gap {
                        continue;
                    }
                    let limit = if solid { 2.0 * o.max_gap } else { reach };
                    let (l, s) = order(&segs[i], &segs[j]);
                    let Some(g) = collinear_gap(l, s, 1.5 * o.tol) else {
                        continue;
                    };
                    if g > limit {
                        continue;
                    }
                    let (a, b) = merged_line(&[&segs[i], &segs[j]]);
                    if let Some(c) = segs[j].small {
                        consumed[c as usize] = true;
                    }
                    if segs[j].lt == LineType::Chain
                        || (solid && segs[j].len() > 0.6 * o.max_gap * 2.0)
                    {
                        segs[i].lt = LineType::Chain;
                    }
                    segs[i].a = a;
                    segs[i].b = b;
                    segs[j].alive = false;
                    any = true;
                }
            }
        }
        if !any {
            break;
        }
    }
}

pub struct HatchOpts {
    pub max_spacing: f64,
    pub min_lines: usize,
    pub axis: bool,
    pub sw: f64,
}

/// ハッチング (平行でほぼ等間隔に並んだ線の束) を検出してグループ番号を振る。
/// 戻り値はグループ数。
pub fn detect_hatch(segs: &mut [Seg], o: &HatchOpts) -> u32 {
    let n = segs.len();
    let off_axis = |s: &Seg| -> bool {
        let d = s.dir();
        d[0].abs().min(d[1].abs()) > 8f64.to_radians().sin()
    };
    let is_cand: Vec<bool> = segs
        .iter()
        .map(|s| {
            s.alive
                && s.lt == LineType::Solid
                && s.len() >= 3.0 * o.sw.max(1.5)
                && (o.axis || off_axis(s))
        })
        .collect();
    let mut grid = Grid::new(o.max_spacing);
    for (i, s) in segs.iter().enumerate() {
        if is_cand[i] {
            grid.insert_seg(s.a, s.b, i as u32);
        }
    }
    let mut uf: Vec<usize> = (0..n).collect();
    fn find(uf: &mut [usize], mut x: usize) -> usize {
        while uf[x] != x {
            uf[x] = uf[uf[x]];
            x = uf[x];
        }
        x
    }
    let sin_tol = 5f64.to_radians().sin();
    let dmin = o.sw.max(1.5);
    let mut cand = Vec::new();
    let mut stamp = vec![usize::MAX; n];
    for i in 0..n {
        if !is_cand[i] {
            continue;
        }
        let (a, b) = (segs[i].a, segs[i].b);
        let di = segs[i].dir();
        let li = segs[i].len();
        let steps = (li / (o.max_spacing * 0.5)).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let p = add(a, mul(sub(b, a), k as f64 / steps as f64));
            grid.near(p, &mut cand);
            for &j in &cand {
                let j = j as usize;
                if j <= i || stamp[j] == i {
                    continue;
                }
                stamp[j] = i;
                let sj = &segs[j];
                if cross(di, sj.dir()).abs() > sin_tol {
                    continue;
                }
                let mid = mul(add(sj.a, sj.b), 0.5);
                let d = line_offset(mid, a, di).abs();
                if d < dmin || d > o.max_spacing {
                    continue;
                }
                let t0 = dot(sub(sj.a, a), di);
                let t1 = dot(sub(sj.b, a), di);
                let (lo, hi) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
                let ov = hi.min(li) - lo.max(0.0);
                if ov < 0.4 * li.min(sj.len()) {
                    continue;
                }
                let (ri, rj) = (find(&mut uf, i), find(&mut uf, j));
                if ri != rj {
                    uf[ri] = rj;
                }
            }
        }
    }
    let mut count: HashMap<usize, usize> = HashMap::new();
    for i in 0..n {
        if is_cand[i] {
            *count.entry(find(&mut uf, i)).or_default() += 1;
        }
    }
    let min_lines = o.min_lines.max(2);
    let mut gid: HashMap<usize, u32> = HashMap::new();
    for i in 0..n {
        if !is_cand[i] {
            continue;
        }
        let r = find(&mut uf, i);
        if count[&r] >= min_lines {
            let next = gid.len() as u32 + 1;
            segs[i].hatch = *gid.entry(r).or_insert(next);
        }
    }
    gid.len() as u32
}

/// 端点の状態。
#[derive(Clone, Copy, PartialEq, Eq)]
enum EndKind {
    /// どこにもつながっていない
    Free,
    /// 他の線の途中に突き当たっている (T 字)
    Tee,
    /// 他の線の端点と合っている (角)。ここで線は「終わっている」とみなす
    Corner,
}

/// 延長すると明らかにつながる線どうしを 1 本にまとめる。
///
/// かすれや他の図形 (壁・文字など) に遮られて途切れた長い線が対象。条件は
/// - 同じ直線に厳密に載っている (長い線どうしなので角度のずれに敏感)
/// - 向かい合う端が、どちらも「角」で終わっていない (角 = 意図して止めた線。開口部の両側の壁など)
/// - すき間が `max_gap` 以下で、かつ長いほうの線より短い。T 字で突き当たっている端を含む場合は `max_gap_tee` 以下
///
/// ハッチングは対象外。
pub fn bridge_collinear(
    segs: &mut [Seg],
    max_gap: f64,
    max_gap_tee: f64,
    min_len: f64,
    tol: f64,
    sw: f64,
) {
    if max_gap <= 0.0 {
        return;
    }
    let r = (1.5 * sw).max(2.5);
    let mut cand = Vec::new();
    for _pass in 0..8 {
        let n = segs.len();
        // 端点の状態
        let mut body = Grid::new((r * 3.0).max(24.0));
        for (i, s) in segs.iter().enumerate() {
            if s.alive {
                body.insert_seg(s.a, s.b, i as u32);
            }
        }
        let mut kind = vec![EndKind::Free; n * 2];
        for i in 0..n {
            if !segs[i].alive {
                continue;
            }
            let di = segs[i].dir();
            for end in 0..2 {
                let e = if end == 0 { segs[i].a } else { segs[i].b };
                body.near(e, &mut cand);
                let mut k = EndKind::Free;
                for &j in &cand {
                    let j = j as usize;
                    if j == i || !segs[j].alive || dist_seg(e, segs[j].a, segs[j].b) > r {
                        continue;
                    }
                    // 同じ向きで重なっている線は「止めている線」ではない
                    if cross(di, segs[j].dir()).abs() < 0.2 {
                        continue;
                    }
                    if dist(e, segs[j].a).min(dist(e, segs[j].b)) <= 2.0 * r {
                        k = EndKind::Corner;
                        break;
                    }
                    k = EndKind::Tee;
                }
                kind[i * 2 + end] = k;
            }
        }
        // 各端点から見た最良の相手
        let mut ends = Grid::new(max_gap);
        for (i, s) in segs.iter().enumerate() {
            if s.alive && s.hatch == 0 && s.len() >= min_len {
                ends.insert(s.a, (i * 2) as u32);
                ends.insert(s.b, (i * 2 + 1) as u32);
            }
        }
        let end_of = |segs: &[Seg], e: usize| -> P {
            if e % 2 == 0 {
                segs[e / 2].a
            } else {
                segs[e / 2].b
            }
        };
        let mut link: Vec<Option<(usize, f64)>> = vec![None; n * 2];
        for e in 0..n * 2 {
            let i = e / 2;
            if !segs[i].alive
                || segs[i].hatch != 0
                || segs[i].len() < min_len
                || kind[e] == EndKind::Corner
            {
                continue;
            }
            let (pe, po) = (end_of(segs, e), end_of(segs, e ^ 1));
            ends.near(pe, &mut cand);
            let mut best: Option<(usize, f64)> = None;
            for &f in &cand {
                let f = f as usize;
                let j = f / 2;
                if j == i || kind[f] == EndKind::Corner || segs[i].lt != segs[j].lt {
                    continue;
                }
                let (pf, pfo) = (end_of(segs, f), end_of(segs, f ^ 1));
                let g = dist(pe, pf);
                let limit = if kind[e] == EndKind::Tee || kind[f] == EndKind::Tee {
                    max_gap_tee
                } else {
                    max_gap
                };
                if g > limit
                    || g > segs[i].len().max(segs[j].len())
                    || dist(po, pf) < g
                    || dist(pe, pfo) < g
                {
                    continue;
                }
                // 互いの延長線上に相手が載っていること。離れるほど位置の誤差は増えるので、
                // 距離に応じて約 0.7° ぶんの角度のずれまで許す (折り目や手描きの線はわずかに曲がる)。
                let on_line = |base: &Seg, other: &Seg| -> bool {
                    let d = base.dir();
                    [other.a, other.b].iter().all(|q| {
                        let t = dot(sub(*q, base.a), d);
                        let beyond = (t - base.len()).max(-t).max(0.0);
                        line_offset(*q, base.a, d).abs() <= tol + 0.012 * beyond
                    })
                };
                // 長いほうの延長線上に短いほうが載っていること。短いほうは向きの誤差が大きいので、
                // 逆向きの確認は「向かい合う端が短いほうの延長のそばにある」ことだけにする。
                let (l, sh) = order(&segs[i], &segs[j]);
                let near_end = if std::ptr::eq(l, &segs[i]) { pe } else { pf };
                if !on_line(l, sh) || line_offset(near_end, sh.a, sh.dir()).abs() > tol + 0.08 * g {
                    continue;
                }
                if best.is_none_or(|b| g < b.1) {
                    best = Some((f, g));
                }
            }
            link[e] = best;
        }
        let mut any = false;
        for e in 0..n * 2 {
            let Some((f, _)) = link[e] else { continue };
            let (i, j) = (e / 2, f / 2);
            if i >= j || link[f].map(|x| x.0) != Some(e) || !segs[i].alive || !segs[j].alive {
                continue;
            }
            let (a, b) = merged_line(&[&segs[i], &segs[j]]);
            segs[i].a = a;
            segs[i].b = b;
            segs[i].small = None;
            segs[j].alive = false;
            any = true;
        }
        if !any {
            break;
        }
    }
}

/// 端点を近くの線に突き合わせる (L 字の角は交点へ、T 字は相手の線上へ)。
pub fn snap_ends(segs: &mut [Seg], r: f64) {
    let orig: Vec<(P, P)> = segs.iter().map(|s| (s.a, s.b)).collect();
    let mut grid = Grid::new((r * 3.0).max(24.0));
    for (i, s) in segs.iter().enumerate() {
        if s.alive {
            grid.insert_seg(s.a, s.b, i as u32);
        }
    }
    let mut cand = Vec::new();
    for i in 0..segs.len() {
        if !segs[i].alive {
            continue;
        }
        let (oa, ob) = orig[i];
        let di = unit(sub(ob, oa));
        for end in 0..2 {
            let e = if end == 0 { oa } else { ob };
            grid.near(e, &mut cand);
            let mut best: Option<(f64, P)> = None;
            for &j in &cand {
                let j = j as usize;
                if j == i || !segs[j].alive {
                    continue;
                }
                let (ja, jb) = orig[j];
                let d = dist_seg(e, ja, jb);
                if d > r {
                    continue;
                }
                let dj = unit(sub(jb, ja));
                if cross(di, dj).abs() < 0.2 {
                    continue;
                }
                let Some(x) = line_intersect(oa, di, ja, dj) else {
                    continue;
                };
                let dx = dist(x, e);
                if dx > 2.5 * r || dist_seg(x, ja, jb) > 2.5 * r {
                    continue;
                }
                if best.is_none_or(|b| dx < b.0) {
                    best = Some((dx, x));
                }
            }
            if let Some((_, x)) = best {
                if end == 0 {
                    segs[i].a = x;
                } else {
                    segs[i].b = x;
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SmallKind {
    /// 文字らしい (曲がった/枝分かれした) 小さな成分
    Glyph,
    /// 単独の短い直線
    Dash,
}

/// 小さな連結成分を近いものどうしでまとめ、文字のかたまりを返す。
/// 各かたまりは (成分番号の列, 文字らしい成分を含むか)。
pub fn cluster_text(
    comps: &[Comp],
    small: &[(u32, SmallKind)],
    sw: f64,
    text_max: f64,
) -> Vec<(Vec<u32>, bool)> {
    let n = small.len();
    let mut uf: Vec<usize> = (0..n).collect();
    fn find(uf: &mut [usize], mut x: usize) -> usize {
        while uf[x] != x {
            uf[x] = uf[uf[x]];
            x = uf[x];
        }
        x
    }
    let bb = |c: u32| -> [f64; 4] {
        let k = &comps[c as usize - 1];
        [
            k.x0 as f64,
            k.y0 as f64,
            k.x1 as f64 + 1.0,
            k.y1 as f64 + 1.0,
        ]
    };
    let mut grid = Grid::new(text_max * 1.5);
    for (i, (c, _)) in small.iter().enumerate() {
        let b = bb(*c);
        grid.insert([(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0], i as u32);
    }
    let mut cand = Vec::new();
    for i in 0..n {
        let bi = bb(small[i].0);
        let si = (bi[2] - bi[0]).max(bi[3] - bi[1]);
        grid.near([(bi[0] + bi[2]) / 2.0, (bi[1] + bi[3]) / 2.0], &mut cand);
        for &j in &cand {
            let j = j as usize;
            if j <= i {
                continue;
            }
            let bj = bb(small[j].0);
            let sj = (bj[2] - bj[0]).max(bj[3] - bj[1]);
            let gx = (bi[0] - bj[2]).max(bj[0] - bi[2]).max(0.0);
            let gy = (bi[1] - bj[3]).max(bj[1] - bi[3]).max(0.0);
            let thr = (0.5 * si.max(sj)).clamp(2.0 * sw, 0.5 * text_max);
            if gx.max(gy) <= thr {
                let (ri, rj) = (find(&mut uf, i), find(&mut uf, j));
                if ri != rj {
                    uf[ri] = rj;
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        groups.entry(find(&mut uf, i)).or_default().push(i);
    }
    let mut out = Vec::new();
    for (_, g) in groups {
        let has_glyph = g.iter().any(|&i| small[i].1 == SmallKind::Glyph);
        out.push((
            g.iter().map(|&i| small[i].0).collect::<Vec<u32>>(),
            has_glyph,
        ));
    }
    out.sort();
    out
}
