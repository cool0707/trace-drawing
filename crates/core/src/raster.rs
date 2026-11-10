//! 8bit グレースケール画像と基本的な画素操作。

#[derive(Clone)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub d: Vec<u8>,
}

impl Gray {
    pub fn new(w: usize, h: usize, fill: u8) -> Self {
        Gray {
            w,
            h,
            d: vec![fill; w * h],
        }
    }
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.d[y * self.w + x]
    }
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, v: u8) {
        self.d[y * self.w + x] = v;
    }
}

/// 画像ファイル (PNG/JPEG/TIFF/BMP/WebP) をデコードしてグレースケール化する。
///
/// 青焼きは青〜紫の線なので、赤チャンネルを重く見たほうが紙とのコントラストが出る。
pub fn decode(bytes: &[u8]) -> Result<Gray, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("画像を読み込めません: {e}"))?;
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let raw = rgb.as_raw();
    let mut d = Vec::with_capacity(w * h);
    for p in raw.chunks_exact(3) {
        d.push(((p[0] as u32 * 128 + p[1] as u32 * 102 + p[2] as u32 * 26) >> 8) as u8);
    }
    Ok(Gray { w, h, d })
}

pub fn encode_png(g: &Gray) -> Vec<u8> {
    let mut out = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut out);
    use image::ImageEncoder;
    enc.write_image(&g.d, g.w as u32, g.h as u32, image::ExtendedColorType::L8)
        .expect("png encode");
    out
}

/// 長辺が `max_side` 以下になるよう面積平均で縮小する。戻り値は (画像, 縮小率)。
pub fn downscale_to(g: &Gray, max_side: usize) -> (Gray, f64) {
    let long = g.w.max(g.h);
    if long <= max_side || max_side == 0 {
        return (g.clone(), 1.0);
    }
    let s = max_side as f64 / long as f64;
    let nw = ((g.w as f64 * s).round() as usize).max(1);
    let nh = ((g.h as f64 * s).round() as usize).max(1);
    let mut out = Gray::new(nw, nh, 0);
    let fx = g.w as f64 / nw as f64;
    let fy = g.h as f64 / nh as f64;
    for y in 0..nh {
        let y0 = (y as f64 * fy) as usize;
        let y1 = (((y + 1) as f64 * fy) as usize).clamp(y0 + 1, g.h);
        for x in 0..nw {
            let x0 = (x as f64 * fx) as usize;
            let x1 = (((x + 1) as f64 * fx) as usize).clamp(x0 + 1, g.w);
            let mut sum = 0u32;
            for yy in y0..y1 {
                let row = &g.d[yy * g.w..yy * g.w + g.w];
                for v in &row[x0..x1] {
                    sum += *v as u32;
                }
            }
            out.d[y * nw + x] = (sum / ((y1 - y0) * (x1 - x0)) as u32) as u8;
        }
    }
    (out, s)
}

/// 時計回りに 90°×k 回転。
pub fn rot90(g: &Gray, k: u8) -> Gray {
    match k % 4 {
        0 => g.clone(),
        2 => {
            let mut d = g.d.clone();
            d.reverse();
            Gray { w: g.w, h: g.h, d }
        }
        k => {
            let (nw, nh) = (g.h, g.w);
            let mut out = Gray::new(nw, nh, 0);
            for y in 0..nh {
                for x in 0..nw {
                    let (sx, sy) = if k == 1 {
                        (y, g.h - 1 - x)
                    } else {
                        (g.w - 1 - y, x)
                    };
                    out.d[y * nw + x] = g.d[sy * g.w + sx];
                }
            }
            out
        }
    }
}

pub fn box_blur3(g: &Gray) -> Gray {
    let (w, h) = (g.w, g.h);
    let mut tmp = vec![0u16; w * h];
    for y in 0..h {
        let r = &g.d[y * w..y * w + w];
        for x in 0..w {
            let a = r[x.saturating_sub(1)] as u16;
            let c = r[(x + 1).min(w - 1)] as u16;
            tmp[y * w + x] = a + r[x] as u16 + c;
        }
    }
    let mut out = Gray::new(w, h, 0);
    for y in 0..h {
        let ya = y.saturating_sub(1);
        let yc = (y + 1).min(h - 1);
        for x in 0..w {
            let s = tmp[ya * w + x] + tmp[y * w + x] + tmp[yc * w + x];
            out.d[y * w + x] = ((s + 4) / 9) as u8;
        }
    }
    out
}

/// 逆写像 `hinv` (出力画素 → 入力画素) でバイリニア補間しながら射影変換する。
pub fn warp(g: &Gray, hinv: &[f64; 9], ow: usize, oh: usize, fill: u8) -> Gray {
    let mut out = Gray::new(ow, oh, fill);
    let (w, h) = (g.w as f64, g.h as f64);
    for y in 0..oh {
        let yf = y as f64;
        let mut px = hinv[1] * yf + hinv[2];
        let mut py = hinv[4] * yf + hinv[5];
        let mut pw = hinv[7] * yf + hinv[8];
        for x in 0..ow {
            let sx = px / pw;
            let sy = py / pw;
            if sx >= 0.0 && sy >= 0.0 && sx < w - 1.0 && sy < h - 1.0 {
                let x0 = sx as usize;
                let y0 = sy as usize;
                let fx = sx - x0 as f64;
                let fy = sy - y0 as f64;
                let i = y0 * g.w + x0;
                let a = g.d[i] as f64;
                let b = g.d[i + 1] as f64;
                let c = g.d[i + g.w] as f64;
                let e = g.d[i + g.w + 1] as f64;
                let v = a * (1.0 - fx) * (1.0 - fy)
                    + b * fx * (1.0 - fy)
                    + c * (1.0 - fx) * fy
                    + e * fx * fy;
                out.d[y * ow + x] = (v + 0.5) as u8;
            }
            px += hinv[0];
            py += hinv[3];
            pw += hinv[6];
        }
    }
    out
}

/// バイリニア補間で 2 倍に拡大する (線が 1 画素しかない低解像度画像用)。
pub fn upscale2(g: &Gray) -> Gray {
    let inv = [0.5, 0.0, -0.25, 0.0, 0.5, -0.25, 0.0, 0.0, 1.0];
    let mut out = warp(g, &inv, g.w * 2, g.h * 2, 255);
    // warp は端の 1 画素を埋めないので最近傍で補う
    for y in 0..out.h {
        for x in 0..out.w {
            let (sx, sy) = (x as f64 * 0.5 - 0.25, y as f64 * 0.5 - 0.25);
            if sx < 0.0 || sy < 0.0 || sx >= g.w as f64 - 1.0 || sy >= g.h as f64 - 1.0 {
                out.d[y * out.w + x] = g.d[(y / 2).min(g.h - 1) * g.w + (x / 2).min(g.w - 1)];
            }
        }
    }
    out
}
