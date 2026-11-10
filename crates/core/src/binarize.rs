//! 汚れた青焼き向けの背景除去と二値化。
//!
//! 1. ブロック単位で「紙の明るさ」を推定して割り算 (フラットフィールド補正)。
//!    シミ・焼けムラ・折り目の影のような低周波の汚れはここで消える。
//! 2. Sauvola の局所しきい値で、かすれた線も濃い線も拾う。

use crate::raster::{box_blur3, Gray};

/// 白地に濃い線になるよう必要なら反転する。`invert`: -1=自動, 0=しない, 1=する。
pub fn fix_polarity(g: &mut Gray, invert: i8) -> bool {
    let do_inv = match invert {
        0 => false,
        1 => true,
        _ => {
            let mut hist = [0usize; 256];
            for v in g.d.iter().step_by(7) {
                hist[*v as usize] += 1;
            }
            let total: usize = hist.iter().sum();
            let mut acc = 0;
            let mut med = 0;
            for (i, c) in hist.iter().enumerate() {
                acc += c;
                if acc * 2 >= total {
                    med = i;
                    break;
                }
            }
            med < 90
        }
    };
    if do_inv {
        for v in g.d.iter_mut() {
            *v = 255 - *v;
        }
    }
    do_inv
}

/// 背景 (紙) の明るさをブロックごとに推定し、背景がほぼ 255 になるよう正規化する。
pub fn flat_field(g: &Gray) -> Gray {
    let (w, h) = (g.w, g.h);
    let bs = (w.max(h) / 64).clamp(16, 128);
    let bw = w.div_ceil(bs);
    let bh = h.div_ceil(bs);
    let mut p = vec![0f32; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let mut hist = [0u32; 256];
            let mut n = 0u32;
            for y in by * bs..((by + 1) * bs).min(h) {
                for v in &g.d[y * w + bx * bs..y * w + ((bx + 1) * bs).min(w)] {
                    hist[*v as usize] += 1;
                    n += 1;
                }
            }
            // 上位 25% 点 = 紙の色 (線が多少混んでいても紙が残る)
            let target = n / 4;
            let mut acc = 0;
            let mut val = 255;
            for i in (0..256).rev() {
                acc += hist[i];
                if acc > target {
                    val = i;
                    break;
                }
            }
            p[by * bw + bx] = val as f32;
        }
    }
    // ベタ塗りなどで紙が見えないブロックは、周囲の中央値で底上げする
    let mut q = p.clone();
    let mut buf = Vec::with_capacity(25);
    for by in 0..bh {
        for bx in 0..bw {
            buf.clear();
            for yy in by.saturating_sub(2)..(by + 3).min(bh) {
                for xx in bx.saturating_sub(2)..(bx + 3).min(bw) {
                    buf.push(p[yy * bw + xx]);
                }
            }
            buf.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let med = buf[buf.len() / 2];
            q[by * bw + bx] = p[by * bw + bx].max(med * 0.9);
        }
    }
    let mut s = q.clone();
    for by in 0..bh {
        for bx in 0..bw {
            let mut sum = 0.0;
            let mut n = 0.0;
            for yy in by.saturating_sub(1)..(by + 2).min(bh) {
                for xx in bx.saturating_sub(1)..(bx + 2).min(bw) {
                    sum += q[yy * bw + xx];
                    n += 1.0;
                }
            }
            s[by * bw + bx] = sum / n;
        }
    }
    let mut out = Gray::new(w, h, 255);
    let half = bs as f32 / 2.0;
    for y in 0..h {
        let fy = ((y as f32 - half) / bs as f32).clamp(0.0, (bh - 1) as f32);
        let y0 = fy as usize;
        let y1 = (y0 + 1).min(bh - 1);
        let ty = fy - y0 as f32;
        for x in 0..w {
            let fx = ((x as f32 - half) / bs as f32).clamp(0.0, (bw - 1) as f32);
            let x0 = fx as usize;
            let x1 = (x0 + 1).min(bw - 1);
            let tx = fx - x0 as f32;
            let bg = (s[y0 * bw + x0] * (1.0 - tx) + s[y0 * bw + x1] * tx) * (1.0 - ty)
                + (s[y1 * bw + x0] * (1.0 - tx) + s[y1 * bw + x1] * tx) * ty;
            let v = g.d[y * w + x] as f32 * 255.0 / bg.max(8.0);
            out.d[y * w + x] = v.min(255.0) as u8;
        }
    }
    out
}

fn otsu(g: &Gray) -> u8 {
    let mut hist = [0f64; 256];
    for v in g.d.iter().step_by(3) {
        hist[*v as usize] += 1.0;
    }
    let total: f64 = hist.iter().sum();
    let sum: f64 = hist.iter().enumerate().map(|(i, c)| i as f64 * c).sum();
    let (mut wb, mut sb, mut best, mut bt) = (0.0, 0.0, -1.0, 128u8);
    for (t, c) in hist.iter().enumerate() {
        wb += c;
        if wb == 0.0 {
            continue;
        }
        let wf = total - wb;
        if wf == 0.0 {
            break;
        }
        sb += t as f64 * c;
        let mb = sb / wb;
        let mf = (sum - sb) / wf;
        let var = wb * wf * (mb - mf) * (mb - mf);
        if var > best {
            best = var;
            bt = t as u8;
        }
    }
    bt
}

/// 幅 2r+1 の箱型平均 (分離型)。
fn box_mean(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0f32; w * h];
    let mut pre = vec![0f64; w.max(h) + 1];
    for y in 0..h {
        let row = &src[y * w..y * w + w];
        for x in 0..w {
            pre[x + 1] = pre[x] + row[x] as f64;
        }
        for x in 0..w {
            let a = x.saturating_sub(r);
            let b = (x + r + 1).min(w);
            tmp[y * w + x] = ((pre[b] - pre[a]) / (b - a) as f64) as f32;
        }
    }
    let mut out = vec![0f32; w * h];
    for x in 0..w {
        for y in 0..h {
            pre[y + 1] = pre[y] + tmp[y * w + x] as f64;
        }
        for y in 0..h {
            let a = y.saturating_sub(r);
            let b = (y + r + 1).min(h);
            out[y * w + x] = ((pre[b] - pre[a]) / (b - a) as f64) as f32;
        }
    }
    out
}

/// 正規化済み画像を二値化する (1 = インク)。`sensitivity` は 0〜1 で大きいほど薄い線も拾う。
///
/// 局所平均との差を、画像全体から推定した紙のノイズ量と比べて判定する。
/// はっきり濃い画素を種にして、薄い画素はそれにつながっている場合だけ採用する
/// (ヒステリシス) ので、かすれた線は途切れにくく、孤立したノイズは拾いにくい。
pub fn threshold(norm: &Gray, sensitivity: f64) -> Gray {
    let (w, h) = (norm.w, norm.h);
    let g = box_blur3(norm);
    let s = sensitivity.clamp(0.0, 1.0) as f32;
    let r = (w.max(h) / 160).clamp(8, 40);
    let v: Vec<f32> = g.d.iter().map(|x| *x as f32).collect();
    let mean = box_mean(&v, w, h, r);
    // 紙のノイズ量 (局所平均との差の MAD)
    let mut hist = [0u32; 512];
    for i in (0..w * h).step_by(5) {
        hist[((mean[i] - v[i]) as i32 + 256).clamp(0, 511) as usize] += 1;
    }
    let total: u32 = hist.iter().sum();
    let pct = |q: f64| -> f32 {
        let mut acc = 0;
        for (i, c) in hist.iter().enumerate() {
            acc += c;
            if acc as f64 >= total as f64 * q {
                return i as f32 - 256.0;
            }
        }
        255.0
    };
    let med = pct(0.5);
    let sigma = ((pct(0.5) - pct(0.25)) * 1.4826).max(1.0);
    let strong = (med + (5.5 - 2.5 * s) * sigma).max(26.0 - 16.0 * s);
    let weak = (med + (3.2 - 1.4 * s) * sigma).max(13.0 - 8.0 * s).max(0.65 * strong);
    // ベタ塗りの内側は局所平均との差が出ないので、紙の明るさとの絶対差でも拾う
    let mut vh = [0u32; 256];
    for x in g.d.iter().step_by(5) {
        vh[*x as usize] += 1;
    }
    let mut acc = 0;
    let mut paper = 255.0f32;
    for (i, c) in vh.iter().enumerate() {
        acc += c;
        if acc * 2 >= total {
            paper = i as f32;
            break;
        }
    }
    let hard = (otsu(&g) as f32).min(150.0).max(paper - (2.2 * strong).max(45.0));
    // 0 = 紙, 1 = 確定, 2 = 保留
    let mut out = Gray::new(w, h, 0);
    let mut stack: Vec<u32> = Vec::new();
    for i in 0..w * h {
        let d = mean[i] - v[i];
        if d > strong || v[i] < hard {
            out.d[i] = 1;
            stack.push(i as u32);
        } else if d > weak {
            out.d[i] = 2;
        }
    }
    while let Some(i) = stack.pop() {
        let i = i as usize;
        let (x, y) = (i % w, i / w);
        for yy in y.saturating_sub(1)..(y + 2).min(h) {
            for xx in x.saturating_sub(1)..(x + 2).min(w) {
                let j = yy * w + xx;
                if out.d[j] == 2 {
                    out.d[j] = 1;
                    stack.push(j as u32);
                }
            }
        }
    }
    for p in out.d.iter_mut() {
        if *p == 2 {
            *p = 0;
        }
    }
    out
}
