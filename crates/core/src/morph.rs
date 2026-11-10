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
        let mut c = Comp { area: 0, x0: u32::MAX, y0: u32::MAX, x1: 0, y1: 0 };
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
    let at = |x: i64, y: i64| -> bool { x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && m.d[y as usize * w + x as usize] == 1 };
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
                next.entry(vid(x, y + 1)).or_default().push(vid(x + 1, y + 1));
            }
            if !at(x - 1, y) {
                next.entry(vid(x, y)).or_default().push(vid(x, y + 1));
            }
            if !at(x + 1, y) {
                next.entry(vid(x + 1, y + 1)).or_default().push(vid(x + 1, y));
            }
        }
    }
    let mut out = Vec::new();
    let keys: Vec<u64> = next.keys().copied().collect();
    for k in keys {
        loop {
            let Some(first) = next.get_mut(&k).and_then(|v| v.pop()) else { break };
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
                        .map(|v| [(v % (w as u64 + 1)) as f64 - 0.5, (v / (w as u64 + 1)) as f64 - 0.5])
                        .collect(),
                );
            }
        }
    }
    out
}
