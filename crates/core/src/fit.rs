//! チェーン (画素列) を直線・円弧・折れ線に当てはめる。
//! 図面用なので曲線でなめらかに近似することはせず、角は角のまま残す。

use crate::geom::*;
use std::f64::consts::PI;

#[derive(Clone, Debug)]
pub enum Prim {
    Seg { a: P, b: P },
    /// 中心 c, 半径 r, 開始角 a0 から sweep (符号付き, 画像座標系) だけ回る
    Arc { c: P, r: f64, a0: f64, sweep: f64 },
    Circle { c: P, r: f64 },
    Poly { pts: Vec<P>, closed: bool },
}

pub struct FitOpts {
    pub eps: f64,
    pub sw: f64,
    pub arcs: bool,
    pub max_r: f64,
}

fn arc_fit(pts: &[P], tol: f64, o: &FitOpts) -> Option<(P, f64, f64, f64)> {
    let (c, r) = fit_circle(pts)?;
    if !(r.is_finite()) || r > o.max_r || r < 1.5 * o.sw.max(2.0) {
        return None;
    }
    let mut sweep = 0.0;
    let a0 = (pts[0][1] - c[1]).atan2(pts[0][0] - c[0]);
    let mut prev = a0;
    let step = (pts.len() / 64).max(1);
    for (i, p) in pts.iter().enumerate() {
        if (dist(*p, c) - r).abs() > tol {
            return None;
        }
        if i % step == 0 || i == pts.len() - 1 {
            let a = (p[1] - c[1]).atan2(p[0] - c[0]);
            let mut d = a - prev;
            while d > PI {
                d -= 2.0 * PI;
            }
            while d < -PI {
                d += 2.0 * PI;
            }
            sweep += d;
            prev = a;
        }
    }
    Some((c, r, a0, sweep))
}

/// 画素列の区間を直線に当てはめ、端点を返す。
fn seg_fit(pts: &[P], o: &FitOpts) -> (P, P) {
    let n = pts.len();
    let (a, b) = (pts[0], pts[n - 1]);
    if n < 8 {
        return (a, b);
    }
    // 交差部は細線化で歪むので端を少し捨てて当てはめる
    let trim = ((o.sw as usize) + 1).min(n / 4);
    let (c, d) = fit_line(&pts[trim..n - trim], None);
    let pa = add(c, mul(d, dot(sub(a, c), d)));
    let pb = add(c, mul(d, dot(sub(b, c), d)));
    if dist(pa, a) > 2.0 * o.eps || dist(pb, b) > 2.0 * o.eps {
        return (a, b);
    }
    (pa, pb)
}

/// 1 本のチェーンをプリミティブ列に変換する。
pub fn fit_chain(pts_in: &[P], closed: bool, o: &FitOpts) -> Vec<Prim> {
    let mut pts: Vec<P> = pts_in.to_vec();
    if pts.len() < 2 {
        return vec![];
    }
    if closed {
        if o.arcs {
            if let Some((c, r)) = fit_circle(&pts) {
                let tol = o.eps.max(0.06 * r);
                if r >= 1.5 * o.sw && r <= o.max_r && pts.iter().all(|p| (dist(*p, c) - r).abs() <= tol) {
                    return vec![Prim::Circle { c, r }];
                }
            }
        }
        // 始点を「最も遠い点」に回して DP が安定するようにする
        let n = pts.len() - 1;
        let far = (0..n).max_by(|&i, &j| dist(pts[i], pts[0]).partial_cmp(&dist(pts[j], pts[0])).unwrap()).unwrap();
        let mut rot: Vec<P> = Vec::with_capacity(n + 1);
        for k in 0..n {
            rot.push(pts[(far + k) % n]);
        }
        rot.push(rot[0]);
        pts = rot;
    }
    let bp = douglas_peucker(&pts, o.eps);
    let np = bp.len() - 1; // 区間数
    let mut out: Vec<Prim> = Vec::new();
    // 区間ごとに arc / seg を決める
    let mut kinds: Vec<Option<(usize, P, f64, f64, f64)>> = vec![None; np]; // arc の開始区間に (終了区間, c, r, a0, sweep)
    let mut used = vec![false; np];
    if o.arcs && np >= 3 {
        let mut i = 0;
        while i + 3 <= np {
            let mut best: Option<(usize, P, f64, f64, f64)> = None;
            let mut j = i + 3;
            while j <= np {
                match arc_fit(&pts[bp[i]..=bp[j]], o.eps * 1.1, o) {
                    Some((c, r, a0, sw)) => {
                        best = Some((j, c, r, a0, sw));
                        j += 1;
                    }
                    None => break,
                }
            }
            if let Some((j, c, r, a0, sweep)) = best {
                if sweep.abs() >= 0.6 {
                    kinds[i] = Some((j, c, r, a0, sweep));
                    for u in used.iter_mut().take(j).skip(i) {
                        *u = true;
                    }
                    i = j;
                    continue;
                }
            }
            i += 1;
        }
    }
    // 直線区間
    let mut segs: Vec<Option<(P, P)>> = vec![None; np];
    for k in 0..np {
        if !used[k] {
            segs[k] = Some(seg_fit(&pts[bp[k]..=bp[k + 1]], o));
        }
    }
    // 隣り合う直線の角は交点に置き直す (角を丸めない)
    for k in 0..np {
        let kn = if k + 1 < np {
            k + 1
        } else if closed && np > 2 {
            0
        } else {
            continue;
        };
        if let (Some((a0, b0)), Some((a1, b1))) = (segs[k], segs[kn]) {
            let (d0, d1) = (unit(sub(b0, a0)), unit(sub(b1, a1)));
            if cross(d0, d1).abs() > 0.25 {
                if let Some(x) = line_intersect(a0, d0, a1, d1) {
                    if dist(x, pts[bp[k + 1]]) <= 2.5 * o.eps + o.sw {
                        segs[k] = Some((a0, x));
                        segs[kn] = Some((x, b1));
                    }
                }
            }
        }
    }
    // 短い区間が続く部分 (自由曲線) は折れ線にまとめる
    let short = 12.0 * o.sw.max(1.5);
    let mut k = 0;
    while k < np {
        if let Some((j, c, r, a0, sweep)) = kinds[k] {
            if sweep.abs() > 2.0 * PI - 0.35 {
                out.push(Prim::Circle { c, r });
            } else {
                out.push(Prim::Arc { c, r, a0, sweep });
            }
            k = j;
            continue;
        }
        let mut j = k;
        while j < np && !used[j] && segs[j].is_some_and(|(a, b)| dist(a, b) < short) {
            j += 1;
        }
        if j - k >= 5 {
            let mut poly = vec![segs[k].unwrap().0];
            for s in segs.iter().take(j).skip(k) {
                poly.push(s.unwrap().1);
            }
            let cl = closed && k == 0 && j == np;
            out.push(Prim::Poly { pts: poly, closed: cl });
            k = j;
            continue;
        }
        if let Some((a, b)) = segs[k] {
            out.push(Prim::Seg { a, b });
        }
        k += 1;
    }
    out
}
