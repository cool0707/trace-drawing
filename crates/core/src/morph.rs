//! 連結成分・距離変換・塗りつぶし領域の抽出。

use crate::geom::P;
use crate::raster::Gray;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct Comp {
    pub area: u32,
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// `value` に等しい画素の連結成分ラベル (1 始まり、0 は対象外)。
pub fn components(b: &Gray, value: u8, conn8: bool) -> (Vec<u32>, Vec<Comp>) {
    let (w, h) = (b.w, b.h);
    let mut lab = vec![0u32; w * h];
    let mut comps = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    for start in 0..w * h {
        if b.d[start] != value || lab[start] != 0 {
            continue;
        }
        let id = comps.len() as u32 + 1;
        let mut c = Comp {
            area: 0,
            x0: u32::MAX,
            y0: u32::MAX,
            x1: 0,
            y1: 0,
        };
        lab[start] = id;
        stack.push(start as u32);
        while let Some(i) = stack.pop() {
            let i = i as usize;
            let (x, y) = (i % w, i / w);
            c.area += 1;
            c.x0 = c.x0.min(x as u32);
            c.x1 = c.x1.max(x as u32);
            c.y0 = c.y0.min(y as u32);
            c.y1 = c.y1.max(y as u32);
            let mut visit = |nx: usize, ny: usize, stack: &mut Vec<u32>| {
                let j = ny * w + nx;
                if b.d[j] == value && lab[j] == 0 {
                    lab[j] = id;
                    stack.push(j as u32);
                }
            };
            let (l, r, u, d) = (x > 0, x + 1 < w, y > 0, y + 1 < h);
            if l {
                visit(x - 1, y, &mut stack);
            }
            if r {
                visit(x + 1, y, &mut stack);
            }
            if u {
                visit(x, y - 1, &mut stack);
            }
            if d {
                visit(x, y + 1, &mut stack);
            }
            if conn8 {
                if l && u {
                    visit(x - 1, y - 1, &mut stack);
                }
                if r && u {
                    visit(x + 1, y - 1, &mut stack);
                }
                if l && d {
                    visit(x - 1, y + 1, &mut stack);
                }
                if r && d {
                    visit(x + 1, y + 1, &mut stack);
                }
            }
        }
        comps.push(c);
    }
    (lab, comps)
}

/// 小さなゴミ (面積 < min_area) を消し、線の中の小さな穴 (面積 <= max_hole) を埋める。
pub fn clean(b: &mut Gray, min_area: u32, max_hole: u32) {
    let (lab, comps) = components(b, 1, true);
    for i in 0..b.d.len() {
        if lab[i] != 0 && comps[lab[i] as usize - 1].area < min_area {
            b.d[i] = 0;
        }
    }
    if max_hole > 0 {
        let (lab, comps) = components(b, 0, false);
        for i in 0..b.d.len() {
            if lab[i] != 0 && comps[lab[i] as usize - 1].area <= max_hole {
                b.d[i] = 1;
            }
        }
    }
}

/// 3-4 チャンファ距離 (単位: 1/3 画素)。`fg` の画素について、非 `fg` 画素までの距離。
/// `border_bg` が真なら画像の外を非 fg とみなす。
pub fn chamfer(b: &Gray, fg: u8, border_bg: bool) -> Vec<u16> {
    let (w, h) = (b.w, b.h);
    const INF: u16 = u16::MAX - 8;
    let edge = if border_bg { 3 } else { INF };
    let mut d = vec![0u16; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if b.d[i] != fg {
                continue;
            }
            let mut m = INF;
            m = m.min(if x > 0 { d[i - 1] + 3 } else { edge });
            if y > 0 {
                m = m.min(d[i - w] + 3);
                m = m.min(if x > 0 { d[i - w - 1] + 4 } else { edge });
                m = m.min(if x + 1 < w { d[i - w + 1] + 4 } else { edge });
            } else {
                m = m.min(edge);
            }
            d[i] = m.min(INF);
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if b.d[i] != fg {
                continue;
            }
            let mut m = d[i];
            m = m.min(if x + 1 < w { d[i + 1] + 3 } else { edge });
            if y + 1 < h {
                m = m.min(d[i + w] + 3);
                m = m.min(if x + 1 < w { d[i + w + 1] + 4 } else { edge });
                m = m.min(if x > 0 { d[i + w - 1] + 4 } else { edge });
            } else {
                m = m.min(edge);
            }
            d[i] = m;
        }
    }
    d
}

/// 代表的な線幅 (画素)。距離変換の尾根上の値の中央値から求める。
pub fn stroke_width(b: &Gray, dt: &[u16]) -> f64 {
    let (w, h) = (b.w, b.h);
    let mut hist = vec![0u32; 256];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w - 1 {
            let i = y * w + x;
            let v = dt[i];
            if v == 0 {
                continue;
            }
            if v >= dt[i - 1] && v >= dt[i + 1] && v >= dt[i - w] && v >= dt[i + w] {
                hist[(v as usize).min(255)] += 1;
            }
        }
    }
    let total: u32 = hist.iter().sum();
    if total == 0 {
        return 2.0;
    }
    let mut acc = 0;
    for (i, c) in hist.iter().enumerate() {
        acc += c;
        if acc * 2 >= total {
            return (2.0 * i as f64 / 3.0 - 1.0).max(1.0);
        }
    }
    2.0
}

/// 太さが `min_width` 以上あるベタ塗り領域を取り出して `b` から取り除く。
/// 戻り値は塗り領域のマスク。
pub fn extract_fills(b: &mut Gray, dt: &[u16], min_width: f64) -> Gray {
    let (w, h) = (b.w, b.h);
    let r3 = (min_width * 1.5) as u16; // 半幅 × 3
    let mut core = Gray::new(w, h, 1);
    let mut any = false;
    for i in 0..w * h {
        if b.d[i] == 1 && dt[i] > r3 {
            core.d[i] = 0;
            any = true;
        }
    }
    let mut fill = Gray::new(w, h, 0);
    if !any {
        return fill;
    }
    // コアからの距離が半幅以内のインク = 開いた (opening) 領域
    let dc = chamfer(&core, 1, false);
    for i in 0..w * h {
        if b.d[i] == 1 && dc[i] <= r3 + 10 {
            fill.d[i] = 1;
        }
    }
    // 小さすぎる塗りは無視 (文字のつぶれなど)
    let (lab, comps) = components(&fill, 1, true);
    let min_area = (min_width * min_width * 3.0) as u32;
    for i in 0..w * h {
        if lab[i] != 0 {
            if comps[lab[i] as usize - 1].area < min_area {
                fill.d[i] = 0;
            } else {
                b.d[i] = 0;
            }
        }
    }
    fill
}

/// マスクの輪郭を画素境界に沿った閉ポリゴン列として取り出す。
pub fn contours(m: &Gray) -> Vec<Vec<P>> {
    let (w, h) = (m.w, m.h);
    let at = |x: i64, y: i64| -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < w
            && (y as usize) < h
            && m.d[y as usize * w + x as usize] == 1
    };
    let vid = |x: i64, y: i64| -> u64 { (y as u64) * (w as u64 + 1) + x as u64 };
    // 内側を左手に見る向きの有向辺
    let mut next: HashMap<u64, Vec<u64>> = HashMap::new();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !at(x, y) {
                continue;
            }
            if !at(x, y - 1) {
                next.entry(vid(x + 1, y)).or_default().push(vid(x, y));
            }
            if !at(x, y + 1) {
                next.entry(vid(x, y + 1))
                    .or_default()
                    .push(vid(x + 1, y + 1));
            }
            if !at(x - 1, y) {
                next.entry(vid(x, y)).or_default().push(vid(x, y + 1));
            }
            if !at(x + 1, y) {
                next.entry(vid(x + 1, y + 1))
                    .or_default()
                    .push(vid(x + 1, y));
            }
        }
    }
    let mut out = Vec::new();
    let keys: Vec<u64> = next.keys().copied().collect();
    for k in keys {
        loop {
            let Some(first) = next.get_mut(&k).and_then(|v| v.pop()) else {
                break;
            };
            let mut poly = vec![k];
            let mut cur = first;
            while cur != k {
                poly.push(cur);
                match next.get_mut(&cur).and_then(|v| v.pop()) {
                    Some(n) => cur = n,
                    None => break,
                }
            }
            if poly.len() >= 4 {
                out.push(
                    poly.iter()
                        .map(|v| {
                            [
                                (v % (w as u64 + 1)) as f64 - 0.5,
                                (v / (w as u64 + 1)) as f64 - 0.5,
                            ]
                        })
                        .collect(),
                );
            }
        }
    }
    out
}

/// 塗り領域に当てはめた単純な図形。
#[derive(Clone, Debug)]
pub enum FillShape {
    /// 矩形 (4 隅、周回順)
    Rect([P; 4]),
    /// 円 (中心, 半径)
    Circle(P, f64),
}

/// 向き `th` の矩形を画素の集まりに当てはめる。戻り値は (IoU, u の範囲, v の範囲)。
/// 辺の位置は断面の画素数が最大の半分を超える所に置くので、辺から突き出た線の付け根などには引きずられない。
fn rect_at(px: &[P], th: f64) -> (f64, [f64; 2], [f64; 2]) {
    let (c, s) = (th.cos(), th.sin());
    let st: Vec<(f64, f64)> = px
        .iter()
        .map(|q| (q[0] * c + q[1] * s, -q[0] * s + q[1] * c))
        .collect();
    let extent = |get: &dyn Fn(&(f64, f64)) -> f64| -> [f64; 2] {
        let lo = st.iter().map(get).fold(f64::MAX, f64::min).floor();
        let hi = st.iter().map(get).fold(f64::MIN, f64::max);
        let mut hist = vec![0u32; (hi - lo) as usize + 1];
        for q in &st {
            hist[(get(q) - lo) as usize] += 1;
        }
        let half = hist.iter().copied().max().unwrap_or(0) / 2;
        let a = hist.iter().position(|&n| n > half).unwrap_or(0);
        let b = hist.iter().rposition(|&n| n > half).unwrap_or(0);
        [lo + a as f64, lo + b as f64 + 1.0]
    };
    let (eu, ev) = (extent(&|q| q.0), extent(&|q| q.1));
    let inside = st
        .iter()
        .filter(|q| q.0 >= eu[0] && q.0 < eu[1] && q.1 >= ev[0] && q.1 < ev[1])
        .count() as f64;
    let area = (eu[1] - eu[0]) * (ev[1] - ev[0]);
    (inside / (px.len() as f64 + area - inside), eu, ev)
}

/// 塗り領域 (ラベル `id` の成分) が矩形か円ならその図形を返す。
///
/// - `snap_deg`: この角度以内の傾きの矩形は水平・垂直にそろえる。
/// - `max_aspect`: これより細長い矩形 (帯) は対象外。
pub fn fit_fill_shape(
    lab: &[u32],
    w: usize,
    id: u32,
    k: &Comp,
    snap_deg: f64,
    max_aspect: f64,
) -> Option<FillShape> {
    const MIN_IOU: f64 = 0.9;
    let mut px: Vec<P> = Vec::with_capacity(k.area as usize);
    let mut edge: Vec<P> = Vec::new();
    let at = |x: i64, y: i64| -> bool {
        x >= k.x0 as i64
            && y >= k.y0 as i64
            && x <= k.x1 as i64
            && y <= k.y1 as i64
            && lab[y as usize * w + x as usize] == id
    };
    for y in k.y0 as i64..=k.y1 as i64 {
        for x in k.x0 as i64..=k.x1 as i64 {
            if !at(x, y) {
                continue;
            }
            px.push([x as f64, y as f64]);
            if !(at(x - 1, y) && at(x + 1, y) && at(x, y - 1) && at(x, y + 1)) {
                edge.push([x as f64, y as f64]);
            }
        }
    }
    let n = px.len() as f64;
    if n < 16.0 {
        return None;
    }

    // --- 円: 重心と、輪郭までの距離の中央値 ---
    let cen = px
        .iter()
        .fold([0.0, 0.0], |a, q| [a[0] + q[0] / n, a[1] + q[1] / n]);
    let mut rs: Vec<f64> = edge
        .iter()
        .map(|q| (q[0] - cen[0]).hypot(q[1] - cen[1]))
        .collect();
    rs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let r = rs[rs.len() / 2] + 0.5;
    let inside = px
        .iter()
        .filter(|q| (q[0] - cen[0]).hypot(q[1] - cen[1]) <= r)
        .count() as f64;
    let circle_iou = inside / (n + std::f64::consts::PI * r * r - inside);

    // --- 矩形: 向きを粗→密に探す (大きな領域は間引いた画素で) ---
    let step = ((n / 20000.0).sqrt().floor() as usize).max(1);
    let coarse: Vec<P> = if step > 1 {
        px.iter()
            .filter(|q| q[0] as usize % step == 0 && q[1] as usize % step == 0)
            .map(|q| [q[0] / step as f64, q[1] / step as f64])
            .collect()
    } else {
        px.clone()
    };
    let mut best = (rect_at(&coarse, 0.0).0, 0.0f64);
    for (range, d) in [(45.0f64, 3.0f64), (3.0, 0.5), (0.5, 0.1)] {
        let c0 = best.1;
        let m = (range / d).round() as i32;
        for i in -m..=m {
            let th = c0 + (i as f64 * d).to_radians();
            let iou = rect_at(&coarse, th).0;
            if iou > best.0 {
                best = (iou, th);
            }
        }
    }
    let mut th = best.1;
    if th.to_degrees().abs() <= snap_deg {
        th = 0.0;
    }
    let (rect_iou, eu, ev) = rect_at(&px, th);

    if circle_iou >= MIN_IOU && circle_iou >= rect_iou {
        return Some(FillShape::Circle(cen, r));
    }
    let (a, b) = (eu[1] - eu[0], ev[1] - ev[0]);
    if rect_iou < MIN_IOU || a.max(b) > max_aspect * a.min(b) {
        return None;
    }
    // 画素 (x, y) は [x-0.5, x+0.5] を占めるので半画素ずらす
    let (c, s) = (th.cos(), th.sin());
    let at_uv = |u: f64, v: f64| -> P { [u * c - v * s - 0.5, u * s + v * c - 0.5] };
    Some(FillShape::Rect([
        at_uv(eu[0], ev[0]),
        at_uv(eu[1], ev[0]),
        at_uv(eu[1], ev[1]),
        at_uv(eu[0], ev[1]),
    ]))
}
