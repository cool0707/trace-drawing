//! 打ち直した文字の大きさ・位置の見積もり。
//!
//! トレースした文字ブロックの枠 (インクの外接矩形) と、打ち込んだ文字列から
//! フォントサイズ (em の高さ) と挿入点 (文字列の左端のベースライン) を決める。
//! 文字ごとにわずかに違う大きさにならないよう、既に使っている大きさか
//! 区切りのよい値 (1, 1.2, 1.5, 2, 2.5, 3, 3.5, 4, 5, 6, 7, 8 × 10ⁿ) に丸める。

use crate::doc::V;

/// 文字の送り幅 [em]。
pub fn advance(c: char) -> f32 {
    let u = c as u32;
    if (0xFF61..=0xFF9F).contains(&u) {
        return 0.5; // 半角カナ
    }
    if u >= 0x2E80 {
        return 1.0; // 漢字・かな・全角記号
    }
    match c {
        ' ' => 0.3,
        'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '\'' | '!' | '|' | 'I' => 0.3,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '-' => 0.4,
        'm' | 'w' | 'M' | 'W' | '@' => 0.85,
        _ => 0.6,
    }
}

/// 文字列のインクの上端・下端 [em] (ベースラインが 0、上が正)。
pub fn ink_extent(s: &str) -> (f32, f32) {
    let (mut top, mut bottom) = (0.0f32, 0.0f32);
    for c in s.chars() {
        let u = c as u32;
        let (t, b) = if is_flat(c) {
            (0.0, 0.0)
        } else if u >= 0x2E80 {
            (0.83, -0.07)
        } else {
            match c {
                'g' | 'j' | 'p' | 'q' | 'y' => (0.52, -0.21),
                'a' | 'c' | 'e' | 'm' | 'n' | 'o' | 'r' | 's' | 'u' | 'v' | 'w' | 'x' | 'z' => {
                    (0.52, 0.0)
                }
                '(' | ')' | '[' | ']' | '{' | '}' | '|' | '/' | '\\' => (0.75, -0.2),
                ',' | ';' => (0.1, -0.15),
                '.' | '-' | '_' | '=' | '~' | '+' | '*' | ':' | '\'' | '"' | '`' | ' ' => {
                    (0.0, 0.0)
                }
                _ => (0.72, 0.0),
            }
        };
        top = top.max(t);
        bottom = bottom.min(b);
    }
    if top < 0.3 {
        top = 0.72; // 記号だけの文字列
    }
    (top, bottom)
}

/// 「一」「ー」のように縦の厚みがほとんどない文字。
fn is_flat(c: char) -> bool {
    matches!(
        c,
        '一' | 'ー'
            | '－'
            | '―'
            | '‐'
            | '＝'
            | '～'
            | '〜'
            | '・'
            | '。'
            | '、'
            | '-'
            | '='
            | '~'
            | '_'
            | '.'
    )
}

pub fn text_advance(s: &str) -> f32 {
    s.chars().map(advance).sum::<f32>().max(0.3)
}

/// 枠と文字列から、丸める前のフォントサイズと回転 [度] を見積もる。
/// 縦長の枠に横書きらしい文字列なら、90° 回転 (下から上へ読む) とみなす。
pub fn estimate(bbox: [f32; 4], s: &str) -> (f32, f32) {
    let (w, h) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
    let (top, bottom) = ink_extent(s);
    let n = s.chars().filter(|c| !c.is_whitespace()).count();
    let rotated = n >= 2 && h > 1.3 * w && text_advance(s) / (top - bottom) > 1.3;
    let rot = if rotated { 90.0 } else { 0.0 };
    (estimate_size(bbox, s, rot), rot)
}

/// 向き `rot` [度] が決まっているときの、丸める前のフォントサイズ。
pub fn estimate_size(bbox: [f32; 4], s: &str, rot: f32) -> f32 {
    let (w, h) = ((bbox[2] - bbox[0]).max(1e-3), (bbox[3] - bbox[1]).max(1e-3));
    let sideways = (45.0..135.0).contains(&rot.rem_euclid(180.0));
    let (along, across) = if sideways { (h, w) } else { (w, h) };
    let (top, bottom) = ink_extent(s);
    // 「一」「ー」のように平たい文字だけなら、高さではなく幅から決める
    if s.chars().filter(|c| !c.is_whitespace()).all(is_flat) {
        along / text_advance(s)
    } else {
        across / (top - bottom)
    }
}

/// 区切りのよい値に丸める。`unit` は図面座標 1 の実寸 [mm] (0 なら図面座標のまま丸める)。
pub fn nice_size(v: f32, unit: f32) -> f32 {
    const STEPS: [f64; 13] = [
        1.0, 1.2, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0,
    ];
    let k = if unit > 0.0 { unit as f64 } else { 1.0 };
    let x = v as f64 * k;
    if x <= 0.0 || !x.is_finite() {
        return v;
    }
    let e = 10f64.powf(x.log10().floor());
    let m = x / e;
    let best = STEPS
        .iter()
        .copied()
        .min_by(|a, b| (m / a).ln().abs().partial_cmp(&(m / b).ln().abs()).unwrap())
        .unwrap();
    (best * e / k) as f32
}

/// 既に使っている大きさのうち、`v` に近い (2 割以内) ものがあればそれを、なければ区切りのよい値を返す。
pub fn pick_size(v: f32, used: &[f32], unit: f32) -> f32 {
    used.iter()
        .copied()
        .filter(|u| *u > 0.0 && (v / u).ln().abs() < 1.2f32.ln())
        .min_by(|a, b| (v / a).ln().abs().partial_cmp(&(v / b).ln().abs()).unwrap())
        .unwrap_or_else(|| nice_size(v, unit))
}

/// 文字列の向き (画面上, y 下向き) の単位ベクトル: (文字列の進む向き, 文字の上向き)。
pub fn axes(rot_deg: f32) -> (V, V) {
    let r = rot_deg.to_radians();
    let (s, c) = r.sin_cos();
    ([c, -s], [-s, -c])
}

/// 枠の中に収まる挿入点 (左端のベースライン)。
pub fn anchor_in(bbox: [f32; 4], s: &str, size: f32, rot: f32) -> V {
    let (top, bottom) = ink_extent(s);
    let len = text_advance(s) * size;
    let (cx, cy) = ((bbox[0] + bbox[2]) / 2.0, (bbox[1] + bbox[3]) / 2.0);
    let (d, up) = axes(rot);
    // 文字列の中心 (進む向きは中央、上下はインクの中央) を枠の中心に合わせる。
    // ただし横書き・縦書きの 90° 単位では、書き出しを枠の端にそろえる (左寄せ)。
    let mid_up = (top + bottom) / 2.0 * size;
    let mut a = [
        cx - d[0] * len / 2.0 - up[0] * mid_up,
        cy - d[1] * len / 2.0 - up[1] * mid_up,
    ];
    let r = rot.rem_euclid(360.0);
    if r.abs() < 0.01 {
        a[0] = bbox[0];
    } else if (r - 90.0).abs() < 0.01 {
        a[1] = bbox[3];
    } else if (r - 180.0).abs() < 0.01 {
        a[0] = bbox[2];
    } else if (r - 270.0).abs() < 0.01 {
        a[1] = bbox[1];
    }
    a
}

/// 挿入点・大きさ・回転から見積もった文字列の外接矩形。
pub fn extent(at: V, s: &str, size: f32, rot: f32) -> [f32; 4] {
    let (top, bottom) = ink_extent(s);
    let len = text_advance(s) * size;
    let (d, up) = axes(rot);
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for (x, y) in [(0.0, bottom), (len, bottom), (len, top), (0.0, top)] {
        let y = y * size;
        let q = [at[0] + d[0] * x + up[0] * y, at[1] + d[1] * x + up[1] * y];
        b = [
            b[0].min(q[0]),
            b[1].min(q[1]),
            b[2].max(q[0]),
            b[3].max(q[1]),
        ];
    }
    b
}

/// 大きさをまとめる: 近い大きさ (最小値から 25% 以内) どうしを 1 つのグループにし、
/// グループの中央値を区切りのよい値に丸めた大きさを返す (入力と同じ並び)。
pub fn cluster_sizes(sizes: &[f32], unit: f32) -> Vec<f32> {
    let mut idx: Vec<usize> = (0..sizes.len()).collect();
    idx.sort_by(|&a, &b| sizes[a].partial_cmp(&sizes[b]).unwrap());
    let mut out = vec![0.0; sizes.len()];
    let mut i = 0;
    while i < idx.len() {
        let lo = sizes[idx[i]];
        let mut j = i;
        while j < idx.len() && sizes[idx[j]] <= lo * 1.25 {
            j += 1;
        }
        let med = sizes[idx[(i + j) / 2]];
        let v = nice_size(med, unit);
        for &k in &idx[i..j] {
            out[k] = v;
        }
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_values() {
        assert_eq!(nice_size(3.3, 1.0), 3.5);
        assert_eq!(nice_size(2.3, 1.0), 2.5);
        assert!((nice_size(33.0, 0.0) - 35.0).abs() < 1e-4);
        assert!((nice_size(0.47, 0.0) - 0.5).abs() < 1e-4);
        // 1 単位 = 0.1 mm のとき 33 単位 = 3.3 mm → 3.5 mm = 35 単位
        assert!((nice_size(33.0, 0.1) - 35.0).abs() < 1e-3);
    }

    #[test]
    fn estimate_cjk_and_digits() {
        // 全角 3 文字、インク高さ 45 → em ≈ 45 / 0.9 = 50
        let (s, r) = estimate([0.0, 0.0, 150.0, 45.0], "平面図");
        assert_eq!(r, 0.0);
        assert!((s - 50.0).abs() < 1.0, "{s}");
        // 数字はインクが em の 0.72
        let (s, _) = estimate([0.0, 0.0, 70.0, 36.0], "7500");
        assert!((s - 50.0).abs() < 1.0, "{s}");
        // 縦長の枠の数字列は 90° 回転
        let (s, r) = estimate([0.0, 0.0, 36.0, 120.0], "7500");
        assert_eq!(r, 90.0);
        assert!((s - 50.0).abs() < 1.0, "{s}");
        // 「一」は幅から
        let (s, _) = estimate([0.0, 0.0, 40.0, 4.0], "一");
        assert!((s - 40.0).abs() < 1.0, "{s}");
        // 打ち込んだ文字が枠より短くても、高さから決める
        let (s, _) = estimate([0.0, 0.0, 156.0, 36.0], "A1");
        assert!((s - 50.0).abs() < 1.0, "{s}");
    }

    #[test]
    fn pick_prefers_used() {
        assert_eq!(pick_size(3.3, &[3.0], 1.0), 3.0);
        assert_eq!(pick_size(3.3, &[5.0], 1.0), 3.5);
        let c = cluster_sizes(&[3.1, 3.4, 3.6, 5.2, 4.9], 1.0);
        assert_eq!(c[0], c[1]);
        assert_eq!(c[1], c[2]);
        assert_eq!(c[0], 3.5);
        assert_eq!(c[3], 5.0);
        assert_eq!(c[4], 5.0);
    }

    #[test]
    fn anchor_and_extent() {
        let bb = [100.0, 200.0, 250.0, 245.0];
        let size = 50.0;
        let at = anchor_in(bb, "平面図", size, 0.0);
        assert_eq!(at[0], 100.0);
        let e = extent(at, "平面図", size, 0.0);
        assert!(((e[1] + e[3]) / 2.0 - 222.5).abs() < 0.5);
        // 90° 回転: 下から上へ
        let bb = [0.0, 0.0, 36.0, 120.0];
        let at = anchor_in(bb, "7500", 50.0, 90.0);
        assert_eq!(at[1], 120.0);
        let e = extent(at, "7500", 50.0, 90.0);
        assert!(((e[0] + e[2]) / 2.0 - 18.0).abs() < 0.5, "{e:?}");
        assert!(e[1] < 10.0);
    }
}
