//! 傾き・歪みの補正。
//!
//! 図面は水平線と垂直線が大半を占めるので、粗くトレースした線分から
//! 「水平線群」「垂直線群」それぞれの消失点を求め、両方が無限遠 (=真の水平・垂直) に
//! 戻るような射影変換を作る。回転・せん断・台形歪み (写真撮影) をまとめて直せる。

use crate::geom::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RectifyMode {
    /// 補正しない
    None,
    /// 回転のみ
    Rotate,
    /// 回転 + せん断
    Affine,
    /// 回転 + せん断 + 台形歪み
    Perspective,
}

fn ang_diff(a: f64, b: f64) -> f64 {
    let mut d = (a - b) % std::f64::consts::PI;
    if d > std::f64::consts::FRAC_PI_2 {
        d -= std::f64::consts::PI;
    }
    if d < -std::f64::consts::FRAC_PI_2 {
        d += std::f64::consts::PI;
    }
    d
}

/// 線分群 (正規化座標) の共通の消失点 (同次座標) を求める。
/// `persp` が偽なら無限遠点 (= 共通の方向) に限定する。
fn vanishing(segs: &[(P, P)], persp: bool) -> Option<[f64; 3]> {
    if segs.len() < 2 {
        return None;
    }
    let lines: Vec<([f64; 3], f64)> = segs
        .iter()
        .map(|(a, b)| {
            let d = unit(sub(*b, *a));
            let n = [-d[1], d[0]];
            ([n[0], n[1], -(n[0] * a[0] + n[1] * a[1])], dist(*a, *b))
        })
        .collect();
    let mut wts: Vec<f64> = lines.iter().map(|l| l.1).collect();
    let mut v = [1.0, 0.0, 0.0];
    for _ in 0..4 {
        let mut m = [[0.0; 3]; 3];
        let mut sw = 0.0;
        for (i, (l, _)) in lines.iter().enumerate() {
            sw += wts[i];
            for r in 0..3 {
                for c in 0..3 {
                    m[r][c] += wts[i] * l[r] * l[c];
                }
            }
        }
        if sw <= 0.0 {
            return None;
        }
        if persp {
            // 根拠が弱いときに極端な台形補正にならないよう、無限遠側に軽く引き寄せる
            m[2][2] += sw * 2e-5;
            v = smallest_eigvec3(&m);
        } else {
            let th = 0.5 * (2.0 * m[0][1]).atan2(m[0][0] - m[1][1]);
            // 2x2 の最小固有ベクトル
            let (c, s) = (th.cos(), th.sin());
            let e1 = m[0][0] * c * c + 2.0 * m[0][1] * c * s + m[1][1] * s * s;
            let e2 = m[0][0] * s * s - 2.0 * m[0][1] * c * s + m[1][1] * c * c;
            v = if e1 < e2 { [c, s, 0.0] } else { [-s, c, 0.0] };
        }
        // 外れ値の重みを落として再計算
        let mut res: Vec<f64> = lines
            .iter()
            .map(|(l, _)| (l[0] * v[0] + l[1] * v[1] + l[2] * v[2]).abs())
            .collect();
        let nrm = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        res.iter_mut().for_each(|r| *r /= nrm);
        let mut sorted = res.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let scale = sorted[sorted.len() / 2].max(1e-5) * 2.5;
        for (i, (_, l)) in lines.iter().enumerate() {
            wts[i] = l / (1.0 + (res[i] / scale).powi(2));
        }
    }
    Some(v)
}

/// 線分から補正用の射影変換 (入力画素 → 補正後画素, 平行移動は未調整) を推定する。
pub fn estimate(segs: &[(P, P)], w: usize, h: usize, mode: RectifyMode) -> Option<M3> {
    if mode == RectifyMode::None {
        return None;
    }
    let long = w.max(h) as f64;
    let min_len = long * 0.03;
    let segs: Vec<(P, P)> = segs
        .iter()
        .filter(|(a, b)| dist(*a, *b) >= min_len)
        .copied()
        .collect();
    if segs.len() < 4 {
        return None;
    }
    // 長さで重み付けした角度ヒストグラム (0.25° 刻み, 180° 周期)
    const NB: usize = 720;
    let mut hist = vec![0.0f64; NB];
    for (a, b) in &segs {
        let d = sub(*b, *a);
        let mut th = d[1].atan2(d[0]).to_degrees();
        if th < 0.0 {
            th += 180.0;
        }
        hist[((th * 4.0) as usize) % NB] += len(d);
    }
    let sm: Vec<f64> = (0..NB)
        .map(|i| (0..7).map(|k| hist[(i + NB + k - 3) % NB]).sum())
        .collect();
    // 直交する 2 方向の合計が最大になる角度
    let mut best = (0.0, 0usize);
    for i in 0..NB / 2 {
        let s = sm[i] + sm[i + NB / 2];
        if s > best.0 {
            best = (s, i);
        }
    }
    let mut base = best.1 as f64 / 4.0 + 0.125; // 度, [0, 90)
    if base > 45.0 {
        base -= 90.0;
    }
    let th_h = base.to_radians();
    let th_v = th_h + std::f64::consts::FRAC_PI_2;
    let c = [w as f64 / 2.0, h as f64 / 2.0];
    let norm = |p: P| -> P { [(p[0] - c[0]) / long, (p[1] - c[1]) / long] };
    let tol = 6.0f64.to_radians();
    let mut gh = Vec::new();
    let mut gv = Vec::new();
    for (a, b) in &segs {
        let d = sub(*b, *a);
        let th = d[1].atan2(d[0]);
        if ang_diff(th, th_h).abs() < tol {
            gh.push((norm(*a), norm(*b)));
        } else if ang_diff(th, th_v).abs() < tol {
            gv.push((norm(*a), norm(*b)));
        }
    }
    let rot_only = |th: f64| -> M3 {
        let (cs, sn) = (th.cos(), th.sin());
        // 中心まわりに -th 回転
        let r = [cs, sn, 0.0, -sn, cs, 0.0, 0.0, 0.0, 1.0];
        let t_in = [1.0, 0.0, -c[0], 0.0, 1.0, -c[1], 0.0, 0.0, 1.0];
        let t_out = [1.0, 0.0, c[0], 0.0, 1.0, c[1], 0.0, 0.0, 1.0];
        m3_mul(&t_out, &m3_mul(&r, &t_in))
    };
    // 回転角は水平・垂直両群の重み付き平均で精密化する
    let refine = || -> f64 {
        let (mut s, mut wsum) = (0.0, 0.0);
        for (g, th0) in [(&gh, th_h), (&gv, th_v)] {
            for (a, b) in g.iter() {
                let d = sub(*b, *a);
                let l = len(d);
                s += ang_diff(d[1].atan2(d[0]), th0) * l;
                wsum += l;
            }
        }
        th_h + if wsum > 0.0 { s / wsum } else { 0.0 }
    };
    if mode == RectifyMode::Rotate || gh.len() < 3 || gv.len() < 3 {
        return Some(rot_only(refine()));
    }
    let persp = mode == RectifyMode::Perspective;
    let (Some(v1), Some(v2)) = (vanishing(&gh, persp), vanishing(&gv, persp)) else {
        return Some(rot_only(refine()));
    };
    // 消失線 l = v1 × v2
    let l = [
        v1[1] * v2[2] - v1[2] * v2[1],
        v1[2] * v2[0] - v1[0] * v2[2],
        v1[0] * v2[1] - v1[1] * v2[0],
    ];
    if l[2].abs() < 1e-9 {
        return Some(rot_only(refine()));
    }
    let (l0, l1) = (l[0] / l[2], l[1] / l[2]);
    // 正規化座標で |l| が大きい = 強すぎる台形歪み → 誤推定とみなして回転のみ
    if (l0 * l0 + l1 * l1).sqrt() > 0.6 {
        return Some(rot_only(refine()));
    }
    let hp: M3 = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, l0, l1, 1.0];
    let dir = |v: [f64; 3]| -> P {
        // hp 適用後は無限遠点なので (x, y) がそのまま方向
        unit([v[0], v[1]])
    };
    let mut d1 = dir(v1);
    let mut d2 = dir(v2);
    if d1[0] < 0.0 {
        d1 = mul(d1, -1.0);
    }
    if d2[1] < 0.0 {
        d2 = mul(d2, -1.0);
    }
    // [d1 d2] の逆行列で d1→x 軸, d2→y 軸
    let det = cross(d1, d2);
    if det.abs() < 0.5 {
        return Some(rot_only(refine()));
    }
    let a: M3 = [
        d2[1] / det,
        -d2[0] / det,
        0.0,
        -d1[1] / det,
        d1[0] / det,
        0.0,
        0.0,
        0.0,
        1.0,
    ];
    let hn = m3_mul(&a, &hp);
    let t_in = [
        1.0 / long,
        0.0,
        -c[0] / long,
        0.0,
        1.0 / long,
        -c[1] / long,
        0.0,
        0.0,
        1.0,
    ];
    let t_out = [long, 0.0, c[0], 0.0, long, c[1], 0.0, 0.0, 1.0];
    Some(m3_mul(&t_out, &m3_mul(&hn, &t_in)))
}

/// 変換後の画像がすべて収まるように平行移動を足し、(変換, 出力幅, 出力高さ) を返す。
pub fn fit_canvas(hm: &M3, w: usize, h: usize) -> (M3, usize, usize) {
    let corners = [
        [0.0, 0.0],
        [w as f64, 0.0],
        [w as f64, h as f64],
        [0.0, h as f64],
    ];
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for c in corners {
        let q = m3_apply(hm, c);
        x0 = x0.min(q[0]);
        y0 = y0.min(q[1]);
        x1 = x1.max(q[0]);
        y1 = y1.max(q[1]);
    }
    let lim = 1.6 * w.max(h) as f64;
    let ow = (x1 - x0).ceil().clamp(1.0, lim) as usize;
    let oh = (y1 - y0).ceil().clamp(1.0, lim) as usize;
    let t = [1.0, 0.0, -x0, 0.0, 1.0, -y0, 0.0, 0.0, 1.0];
    (m3_mul(&t, hm), ow, oh)
}

/// 4 隅 (左上, 右上, 右下, 左下の順でなくてもよい) を長方形に写す変換。
pub fn from_corners(pts: &[P; 4]) -> Option<M3> {
    // 重心まわりの角度で並べ替え → 左上から時計回り
    let c = [
        pts.iter().map(|p| p[0]).sum::<f64>() / 4.0,
        pts.iter().map(|p| p[1]).sum::<f64>() / 4.0,
    ];
    let mut s = *pts;
    s.sort_by(|a, b| {
        (a[1] - c[1])
            .atan2(a[0] - c[0])
            .partial_cmp(&(b[1] - c[1]).atan2(b[0] - c[0]))
            .unwrap()
    });
    // atan2 昇順 (画像座標) は 左上(-135°) → 右上(-45°) → 右下(45°) → 左下(135°)
    let wd = (dist(s[0], s[1]) + dist(s[3], s[2])) / 2.0;
    let ht = (dist(s[0], s[3]) + dist(s[1], s[2])) / 2.0;
    let (x0, y0) = (c[0] - wd / 2.0, c[1] - ht / 2.0);
    let dst = [[x0, y0], [x0 + wd, y0], [x0 + wd, y0 + ht], [x0, y0 + ht]];
    homography_4pt(&s, &dst)
}
