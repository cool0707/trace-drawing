//! 細線化 (Zhang-Suen) と、骨格画素 → 節点間チェーンへの分解。

use crate::geom::P;
use crate::raster::Gray;

/// 細線化した骨格。周囲に 1 画素の余白を持つ。
pub struct Skel {
    pub w: usize,
    pub h: usize,
    pub d: Vec<u8>,
}

const OFF: [(i32, i32); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

pub fn thin(b: &Gray) -> Skel {
    let (w, h) = (b.w + 2, b.h + 2);
    let mut d = vec![0u8; w * h];
    let mut active: Vec<u32> = Vec::new();
    for y in 0..b.h {
        for x in 0..b.w {
            if b.d[y * b.w + x] == 1 {
                let i = (y + 1) * w + x + 1;
                d[i] = 1;
                active.push(i as u32);
            }
        }
    }
    let wi = w as isize;
    let nb: [isize; 8] = [-wi, -wi + 1, 1, wi + 1, wi, wi - 1, -1, -wi - 1];
    let mut rm: Vec<u32> = Vec::new();
    loop {
        let mut changed = false;
        for step in 0..2 {
            rm.clear();
            for &i in &active {
                let i = i as isize;
                if d[i as usize] == 0 {
                    continue;
                }
                let mut p = [0u8; 8];
                let mut bsum = 0;
                for k in 0..8 {
                    p[k] = d[(i + nb[k]) as usize];
                    bsum += p[k];
                }
                if !(2..=6).contains(&bsum) {
                    continue;
                }
                let mut a = 0;
                for k in 0..8 {
                    if p[k] == 0 && p[(k + 1) % 8] == 1 {
                        a += 1;
                    }
                }
                if a != 1 {
                    continue;
                }
                let ok = if step == 0 {
                    p[0] * p[2] * p[4] == 0 && p[2] * p[4] * p[6] == 0
                } else {
                    p[0] * p[2] * p[6] == 0 && p[0] * p[4] * p[6] == 0
                };
                if ok {
                    rm.push(i as u32);
                }
            }
            if !rm.is_empty() {
                changed = true;
                for &i in &rm {
                    d[i as usize] = 0;
                }
            }
        }
        active.retain(|&i| d[i as usize] == 1);
        if !changed {
            break;
        }
    }
    let mut s = Skel { w, h, d };
    s.remove_staircase();
    s
}

impl Skel {
    /// 階段状の余分な画素を除いて厳密に 1 画素幅にする。
    pub fn remove_staircase(&mut self) {
        let w = self.w as isize;
        for pass in 0..2 {
            let mut rm = Vec::new();
            for i in (self.w + 1)..(self.d.len() - self.w - 1) {
                if self.d[i] == 0 {
                    continue;
                }
                let g = |o: isize| self.d[(i as isize + o) as usize] == 1;
                let (n, s, e, wst) = (g(-w), g(w), g(1), g(-1));
                let (ne, nw, se, sw) = (g(-w + 1), g(-w - 1), g(w + 1), g(w - 1));
                let del = if pass == 0 {
                    n && ((e && !ne && !sw && (!wst || !s)) || (wst && !nw && !se && (!e || !s)))
                } else {
                    s && ((e && !se && !nw && (!wst || !n)) || (wst && !sw && !ne && (!e || !n)))
                };
                if del {
                    rm.push(i);
                }
            }
            for i in rm {
                // 直前の削除で条件が変わっていないか確認してから消す
                let g = |o: isize| self.d[(i as isize + o) as usize] == 1;
                let (n, s, e, wst) = (g(-w), g(w), g(1), g(-1));
                if (n || s) && (e || wst) {
                    self.d[i] = 0;
                }
            }
        }
    }

    /// m 隣接 (4 近傍経由で行ける斜めは無視) の近傍。
    fn mneigh(&self, i: usize, out: &mut Vec<usize>) {
        out.clear();
        let w = self.w as isize;
        let ii = i as isize;
        let g = |dx: i32, dy: i32| self.d[(ii + dy as isize * w + dx as isize) as usize] == 1;
        for (dx, dy) in OFF {
            if !g(dx, dy) {
                continue;
            }
            if dx != 0 && dy != 0 && (g(dx, 0) || g(0, dy)) {
                continue;
            }
            out.push((ii + dy as isize * w + dx as isize) as usize);
        }
    }
}

pub struct Chain {
    pub pts: Vec<P>,
    pub closed: bool,
    /// 各端が自由端 (どこにも接続していない) か
    pub free: [bool; 2],
    pub width: f64,
    pub comp: u32,
}

struct Raw {
    pix: Vec<usize>,
    /// 端の節点 (クラスタ番号)。自由端・閉ループは usize::MAX
    node: [usize; 2],
    closed: bool,
}

fn build_raw(s: &Skel) -> (Vec<Raw>, Vec<P>, Vec<usize>) {
    let n = s.d.len();
    let mut deg = vec![0u8; n];
    let mut nb = Vec::with_capacity(8);
    for i in 0..n {
        if s.d[i] == 1 {
            s.mneigh(i, &mut nb);
            deg[i] = nb.len() as u8;
        }
    }
    // 分岐画素をクラスタにまとめる
    let mut cl = vec![usize::MAX; n];
    let mut centers: Vec<P> = Vec::new();
    let mut rep: Vec<usize> = Vec::new();
    let mut stack = Vec::new();
    for i in 0..n {
        if deg[i] < 3 || cl[i] != usize::MAX {
            continue;
        }
        let id = centers.len();
        let (mut sx, mut sy, mut cnt) = (0.0, 0.0, 0.0);
        cl[i] = id;
        stack.push(i);
        while let Some(j) = stack.pop() {
            sx += (j % s.w) as f64;
            sy += (j / s.w) as f64;
            cnt += 1.0;
            s.mneigh(j, &mut nb);
            for &k in &nb {
                if deg[k] >= 3 && cl[k] == usize::MAX {
                    cl[k] = id;
                    stack.push(k);
                }
            }
        }
        centers.push([sx / cnt - 1.0, sy / cnt - 1.0]);
        rep.push(i);
    }
    let mut visited = vec![false; n];
    let mut raws = Vec::new();
    let walk = |start: usize,
                prev: usize,
                visited: &mut Vec<bool>,
                nb: &mut Vec<usize>|
     -> (Vec<usize>, usize) {
        let mut pix = vec![start];
        visited[start] = true;
        let (mut cur, mut prev) = (start, prev);
        loop {
            s.mneigh(cur, nb);
            let mut next = usize::MAX;
            // 分岐画素を優先 (1 画素だけのチェーンで両端が分岐の場合に対応)
            for &k in nb.iter() {
                if k != prev && deg[k] >= 3 {
                    next = k;
                    break;
                }
            }
            if next == usize::MAX {
                for &k in nb.iter() {
                    if k != prev && !visited[k] {
                        next = k;
                        break;
                    }
                }
            }
            if next == usize::MAX {
                return (pix, usize::MAX);
            }
            if deg[next] >= 3 {
                return (pix, cl[next]);
            }
            visited[next] = true;
            pix.push(next);
            prev = cur;
            cur = next;
        }
    };
    // 分岐から出るチェーン
    for i in 0..n {
        if deg[i] < 3 {
            continue;
        }
        s.mneigh(i, &mut nb);
        let starts: Vec<usize> = nb.clone();
        for q in starts {
            if deg[q] >= 3 || visited[q] {
                continue;
            }
            let (pix, end) = walk(q, i, &mut visited, &mut nb);
            raws.push(Raw {
                pix,
                node: [cl[i], end],
                closed: false,
            });
        }
    }
    // 自由端どうしを結ぶチェーン
    for i in 0..n {
        if s.d[i] == 1 && deg[i] == 1 && !visited[i] {
            let (pix, end) = walk(i, usize::MAX, &mut visited, &mut nb);
            raws.push(Raw {
                pix,
                node: [usize::MAX, end],
                closed: false,
            });
        }
    }
    // 閉ループ
    for i in 0..n {
        if s.d[i] == 1 && deg[i] == 2 && !visited[i] {
            let (pix, _) = walk(i, usize::MAX, &mut visited, &mut nb);
            if pix.len() >= 4 {
                raws.push(Raw {
                    pix,
                    node: [usize::MAX; 2],
                    closed: true,
                });
            }
        }
    }
    (raws, centers, rep)
}

/// 骨格をチェーンに分解する。細線化で生じる短いヒゲは取り除く。
/// `dt` はインクのチャンファ距離、`labels` は連結成分ラベル (どちらも余白なしの元画像座標)。
pub fn chains(s: &mut Skel, dt: &[u16], labels: &[u32]) -> Vec<Chain> {
    let (pw, ow) = (s.w, s.w - 2);
    let unpad = move |i: usize| -> usize { (i / pw - 1) * ow + (i % pw - 1) };
    // ヒゲの除去 (2 回まで)
    for _ in 0..2 {
        let (raws, _, rep) = build_raw(s);
        let mut removed = false;
        let mut erase = Vec::new();
        for r in &raws {
            if r.closed {
                continue;
            }
            let (free0, free1) = (r.node[0] == usize::MAX, r.node[1] == usize::MAX);
            if free0 == free1 {
                continue;
            }
            let node = if free0 { r.node[1] } else { r.node[0] };
            let half = dt[unpad(rep[node])] as f64 / 3.0;
            if (r.pix.len() as f64) < 1.5 * half + 2.0 {
                erase.extend_from_slice(&r.pix);
                removed = true;
            }
        }
        if !removed {
            break;
        }
        for i in erase {
            s.d[i] = 0;
        }
        s.remove_staircase();
    }
    let (raws, centers, _) = build_raw(s);
    let mut out = Vec::with_capacity(raws.len());
    for r in raws {
        let mut pts: Vec<P> = Vec::with_capacity(r.pix.len() + 2);
        if r.node[0] != usize::MAX {
            pts.push(centers[r.node[0]]);
        }
        let mut wsum = 0.0;
        for &i in &r.pix {
            pts.push([(i % s.w) as f64 - 1.0, (i / s.w) as f64 - 1.0]);
            wsum += dt[unpad(i)] as f64;
        }
        if r.node[1] != usize::MAX {
            pts.push(centers[r.node[1]]);
        }
        if r.closed {
            pts.push(pts[0]);
        }
        let width = (2.0 * wsum / r.pix.len() as f64 / 3.0 - 1.0).max(1.0);
        out.push(Chain {
            pts,
            closed: r.closed,
            free: [
                r.node[0] == usize::MAX && !r.closed,
                r.node[1] == usize::MAX && !r.closed,
            ],
            width,
            comp: labels[unpad(r.pix[0])],
        });
    }
    out
}
