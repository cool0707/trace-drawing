//! 2D 幾何ユーティリティ。

pub type P = [f64; 2];

#[inline]
pub fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
#[inline]
pub fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
#[inline]
pub fn mul(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s]
}
#[inline]
pub fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
#[inline]
pub fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
#[inline]
pub fn len(a: P) -> f64 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}
#[inline]
pub fn dist(a: P, b: P) -> f64 {
    len(sub(a, b))
}
pub fn unit(a: P) -> P {
    let l = len(a);
    if l < 1e-12 {
        [1.0, 0.0]
    } else {
        [a[0] / l, a[1] / l]
    }
}

/// 点 p から線分 ab までの距離。
pub fn dist_seg(p: P, a: P, b: P) -> f64 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    if l2 < 1e-12 {
        return dist(p, a);
    }
    let t = (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0);
    dist(p, add(a, mul(ab, t)))
}

/// 点 p から直線 (a を通り方向 d, |d|=1) までの符号付き距離。
#[inline]
pub fn line_offset(p: P, a: P, d: P) -> f64 {
    cross(d, sub(p, a))
}

/// 2 直線 (点+方向) の交点。平行なら None。
pub fn line_intersect(a: P, da: P, b: P, db: P) -> Option<P> {
    let den = cross(da, db);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = cross(sub(b, a), db) / den;
    Some(add(a, mul(da, t)))
}

/// 重み付き全最小二乗による直線当てはめ。(重心, 単位方向) を返す。
pub fn fit_line(pts: &[P], wts: Option<&[f64]>) -> (P, P) {
    let mut sw = 0.0;
    let mut c = [0.0, 0.0];
    for (i, p) in pts.iter().enumerate() {
        let w = wts.map_or(1.0, |w| w[i]);
        sw += w;
        c[0] += w * p[0];
        c[1] += w * p[1];
    }
    if sw <= 0.0 {
        return ([0.0, 0.0], [1.0, 0.0]);
    }
    c = [c[0] / sw, c[1] / sw];
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for (i, p) in pts.iter().enumerate() {
        let w = wts.map_or(1.0, |w| w[i]);
        let dx = p[0] - c[0];
        let dy = p[1] - c[1];
        sxx += w * dx * dx;
        sxy += w * dx * dy;
        syy += w * dy * dy;
    }
    let th = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    (c, [th.cos(), th.sin()])
}

/// 円の代数的当てはめ (Kåsa)。(中心, 半径)。
pub fn fit_circle(pts: &[P]) -> Option<(P, f64)> {
    let n = pts.len() as f64;
    if pts.len() < 3 {
        return None;
    }
    let (mut mx, mut my) = (0.0, 0.0);
    for p in pts {
        mx += p[0];
        my += p[1];
    }
    mx /= n;
    my /= n;
    let (mut suu, mut suv, mut svv, mut suuu, mut svvv, mut suvv, mut svuu) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for p in pts {
        let u = p[0] - mx;
        let v = p[1] - my;
        suu += u * u;
        suv += u * v;
        svv += v * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }
    let den = suu * svv - suv * suv;
    if den.abs() < 1e-9 {
        return None;
    }
    let r1 = 0.5 * (suuu + suvv);
    let r2 = 0.5 * (svvv + svuu);
    let uc = (r1 * svv - r2 * suv) / den;
    let vc = (r2 * suu - r1 * suv) / den;
    let r = (uc * uc + vc * vc + (suu + svv) / n).sqrt();
    Some(([uc + mx, vc + my], r))
}

/// Douglas-Peucker。残す頂点のインデックスを返す (両端を含む)。
pub fn douglas_peucker(pts: &[P], eps: f64) -> Vec<usize> {
    let n = pts.len();
    if n < 3 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j <= i + 1 {
            continue;
        }
        let (a, b) = (pts[i], pts[j]);
        let mut best = 0.0;
        let mut bi = i;
        for (k, p) in pts.iter().enumerate().take(j).skip(i + 1) {
            let d = dist_seg(*p, a, b);
            if d > best {
                best = d;
                bi = k;
            }
        }
        if best > eps {
            keep[bi] = true;
            stack.push((i, bi));
            stack.push((bi, j));
        }
    }
    (0..n).filter(|&i| keep[i]).collect()
}

// ---- 3x3 行列 (射影変換) ----

pub type M3 = [f64; 9];
pub const IDENT: M3 = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

pub fn m3_mul(a: &M3, b: &M3) -> M3 {
    let mut o = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            o[r * 3 + c] = a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c];
        }
    }
    o
}

pub fn m3_inv(m: &M3) -> Option<M3> {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det.abs() < 1e-14 {
        return None;
    }
    let id = 1.0 / det;
    Some([
        (m[4] * m[8] - m[5] * m[7]) * id,
        (m[2] * m[7] - m[1] * m[8]) * id,
        (m[1] * m[5] - m[2] * m[4]) * id,
        (m[5] * m[6] - m[3] * m[8]) * id,
        (m[0] * m[8] - m[2] * m[6]) * id,
        (m[2] * m[3] - m[0] * m[5]) * id,
        (m[3] * m[7] - m[4] * m[6]) * id,
        (m[1] * m[6] - m[0] * m[7]) * id,
        (m[0] * m[4] - m[1] * m[3]) * id,
    ])
}

pub fn m3_apply(m: &M3, p: P) -> P {
    let w = m[6] * p[0] + m[7] * p[1] + m[8];
    [
        (m[0] * p[0] + m[1] * p[1] + m[2]) / w,
        (m[3] * p[0] + m[4] * p[1] + m[5]) / w,
    ]
}

/// 対称 3x3 行列の最小固有値に対応する固有ベクトル (Jacobi 法)。
pub fn smallest_eigvec3(m: &[[f64; 3]; 3]) -> [f64; 3] {
    let mut a = *m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..60 {
        let mut off = 0.0;
        let (mut p, mut q) = (0, 1);
        for i in 0..3 {
            for j in (i + 1)..3 {
                if a[i][j].abs() > off {
                    off = a[i][j].abs();
                    p = i;
                    q = j;
                }
            }
        }
        if off < 1e-15 {
            break;
        }
        let th = 0.5 * (2.0 * a[p][q]).atan2(a[q][q] - a[p][p]);
        let (c, s) = (th.cos(), th.sin());
        for k in 0..3 {
            let (akp, akq) = (a[k][p], a[k][q]);
            a[k][p] = c * akp - s * akq;
            a[k][q] = s * akp + c * akq;
        }
        for k in 0..3 {
            let (apk, aqk) = (a[p][k], a[q][k]);
            a[p][k] = c * apk - s * aqk;
            a[q][k] = s * apk + c * aqk;
        }
        for row in v.iter_mut() {
            let (vp, vq) = (row[p], row[q]);
            row[p] = c * vp - s * vq;
            row[q] = s * vp + c * vq;
        }
    }
    let mut k = 0;
    for i in 1..3 {
        if a[i][i] < a[k][k] {
            k = i;
        }
    }
    [v[0][k], v[1][k], v[2][k]]
}

/// 4 点対応から射影変換を求める (src[i] → dst[i])。
pub fn homography_4pt(src: &[P; 4], dst: &[P; 4]) -> Option<M3> {
    let mut a = [[0.0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = (src[i][0], src[i][1]);
        let (u, v) = (dst[i][0], dst[i][1]);
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    // ガウス消去 (部分ピボット)
    for c in 0..8 {
        let mut piv = c;
        for r in c + 1..8 {
            if a[r][c].abs() > a[piv][c].abs() {
                piv = r;
            }
        }
        if a[piv][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, piv);
        for r in 0..8 {
            if r != c {
                let f = a[r][c] / a[c][c];
                if f != 0.0 {
                    for k in c..9 {
                        a[r][k] -= f * a[c][k];
                    }
                }
            }
        }
    }
    let mut h = [0.0; 9];
    for i in 0..8 {
        h[i] = a[i][8] / a[i][i];
    }
    h[8] = 1.0;
    Some(h)
}
