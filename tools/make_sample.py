#!/usr/bin/env python3
"""テスト用に「汚い青焼き図面」風の画像を合成する。

  python3 tools/make_sample.py out.png [--clean]

きれいな線画を描いたあと、傾き・台形歪み・ムラ・シミ・かすれ・ノイズを加える。
"""
import math
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H = 2400, 1700
rng = np.random.default_rng(7)


def font(size):
    for path in ("/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf", "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf"):
        try:
            return ImageFont.truetype(path, size)
        except OSError:
            pass
    return ImageFont.load_default()


def dashed(d, a, b, dash, gap, width, pattern=None):
    (x0, y0), (x1, y1) = a, b
    L = math.hypot(x1 - x0, y1 - y0)
    ux, uy = (x1 - x0) / L, (y1 - y0) / L
    pattern = pattern or [dash, gap]
    t, i = 0.0, 0
    while t < L:
        seg = pattern[i % len(pattern)]
        if i % 2 == 0:
            e = min(L, t + seg)
            d.line([(x0 + ux * t, y0 + uy * t), (x0 + ux * e, y0 + uy * e)], fill=0, width=width)
        t += seg
        i += 1


def hatch(d, box, spacing, width):
    x0, y0, x1, y1 = box
    k = -(y1 - y0)
    while k < (x1 - x0):
        # 45° の線を矩形でクリップ
        ax, ay = x0 + k, y1
        bx, by = x0 + k + (y1 - y0), y0
        if ax < x0:
            ay -= x0 - ax
            ax = x0
        if bx > x1:
            by += bx - x1
            bx = x1
        if bx > ax:
            d.line([(ax, ay), (bx, by)], fill=0, width=width)
        k += spacing


def draw():
    img = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(img)
    # 図面枠
    d.rectangle([80, 80, W - 80, H - 80], outline=0, width=5)
    # 表題欄
    d.rectangle([W - 680, H - 260, W - 80, H - 80], outline=0, width=3)
    d.line([(W - 680, H - 170), (W - 80, H - 170)], fill=0, width=2)
    d.line([(W - 420, H - 260), (W - 420, H - 80)], fill=0, width=2)
    d.text((W - 660, H - 245), "工事名称", font=font(34), fill=0)
    d.text((W - 400, H - 245), "事務所新築工事", font=font(34), fill=0)
    d.text((W - 660, H - 150), "図面名称", font=font(34), fill=0)
    d.text((W - 400, H - 150), "1階平面図 1/100", font=font(34), fill=0)

    # 外壁 (二重線 + ハッチング)
    ox0, oy0, ox1, oy1 = 300, 300, 1700, 1250
    t = 40
    d.rectangle([ox0, oy0, ox1, oy1], outline=0, width=4)
    d.rectangle([ox0 + t, oy0 + t, ox1 - t, oy1 - t], outline=0, width=4)
    hatch(d, (ox0, oy0, ox1, oy0 + t), 14, 2)
    hatch(d, (ox0, oy1 - t, ox1, oy1), 14, 2)
    hatch(d, (ox0, oy0 + t, ox0 + t, oy1 - t), 14, 2)
    hatch(d, (ox1 - t, oy0 + t, ox1, oy1 - t), 14, 2)
    # 間仕切り
    d.line([(900, oy0 + t), (900, oy1 - t)], fill=0, width=4)
    d.line([(920, oy0 + t), (920, oy1 - t)], fill=0, width=4)
    d.line([(920, 800), (ox1 - t, 800)], fill=0, width=4)
    d.line([(920, 820), (ox1 - t, 820)], fill=0, width=4)
    # 柱 (ベタ塗り)
    for cx, cy in [(ox0 + 20, oy0 + 20), (ox1 - 20, oy0 + 20), (ox0 + 20, oy1 - 20), (ox1 - 20, oy1 - 20), (910, oy0 + 20)]:
        d.rectangle([cx - 32, cy - 32, cx + 32, cy + 32], fill=0)
    # 通り芯 (一点鎖線) と通り芯記号
    for x in (ox0 + 20, 910, ox1 - 20):
        dashed(d, (x, 170), (x, 1400), 0, 0, 2, pattern=[70, 12, 8, 12])
        d.ellipse([x - 38, 1400, x + 38, 1476], outline=0, width=3)
    for i, x in enumerate((ox0 + 20, 910, ox1 - 20)):
        d.text((x - 14, 1414), "XYZ"[i], font=font(44), fill=0)
    for y in (oy0 + 20, oy1 - 20):
        dashed(d, (170, y), (1850, y), 0, 0, 2, pattern=[70, 12, 8, 12])
    # 隠れ線 (破線)
    dashed(d, (400, 520), (860, 520), 26, 12, 3)
    dashed(d, (400, 520), (400, 900), 26, 12, 3)
    dashed(d, (1000, 420), (1600, 760), 26, 12, 3)
    # 扉 (円弧)
    d.line([(1100, 820), (1100, 960)], fill=0, width=3)
    d.arc([960, 680, 1240, 960], 0, 90, fill=0, width=2)
    d.arc([420, 1070, 700, 1350], 270, 360, fill=0, width=2)
    d.line([(560, 1070), (560, 1210)], fill=0, width=3)
    # 設備 (円)
    d.ellipse([1300, 950, 1420, 1070], outline=0, width=3)
    d.ellipse([1335, 985, 1385, 1035], outline=0, width=3)
    # 斜めの線・階段
    for i in range(9):
        y = 400 + i * 36
        d.line([(1250, y), (1560, y)], fill=0, width=2)
    d.rectangle([1250, 400, 1560, 688], outline=0, width=3)
    d.line([(1250, 688), (1560, 400)], fill=0, width=2)
    # 寸法線
    d.line([(ox0, 220), (ox1, 220)], fill=0, width=2)
    for x in (ox0, 910, ox1):
        d.line([(x, 195), (x, 245)], fill=0, width=2)
        d.line([(x - 12, 232), (x + 12, 208)], fill=0, width=2)
    d.text((560, 170), "6,100", font=font(40), fill=0)
    d.text((1260, 170), "7,900", font=font(40), fill=0)
    # 室名など
    d.text((500, 650), "事務室", font=font(56), fill=0)
    d.text((1120, 560), "階段室", font=font(48), fill=0)
    d.text((1250, 1120), "機械室", font=font(48), fill=0)
    d.text((1900, 400), "特記事項", font=font(40), fill=0)
    for i, s in enumerate(["1. 寸法は現場確認のこと", "2. 壁厚はW=150とする", "3. 仕上表による"]):
        d.text((1900, 470 + i * 54), s, font=font(30), fill=0)
    return img


def degrade(img):
    # 傾き + 台形歪み
    ang = math.radians(2.3)
    c, s = math.cos(ang), math.sin(ang)
    cx, cy = W / 2, H / 2
    px, py = 2.2e-5, -1.5e-5

    def fwd(x, y):
        x, y = x - cx, y - cy
        x, y = c * x - s * y, s * x + c * y
        w = 1 + px * x + py * y
        return x / w + cx, y / w + cy

    # PIL は出力→入力の係数を取るので 4 点から逆変換を解く
    src = [(0, 0), (W, 0), (W, H), (0, H)]
    dst = [fwd(*p) for p in src]
    A, b = [], []
    for (u, v), (x, y) in zip(dst, src):
        A.append([u, v, 1, 0, 0, 0, -x * u, -x * v]); b.append(x)
        A.append([0, 0, 0, u, v, 1, -y * u, -y * v]); b.append(y)
    coef = np.linalg.solve(np.array(A, float), np.array(b, float))
    img = img.transform((W, H), Image.PERSPECTIVE, tuple(coef), Image.BICUBIC, fillcolor=255)
    img = img.filter(ImageFilter.GaussianBlur(0.9))
    ink = 1.0 - np.asarray(img, float) / 255.0

    yy, xx = np.mgrid[0:H, 0:W]
    # かすれ: 場所によって線の濃さが変わる
    low = rng.normal(0, 1, (H // 80 + 2, W // 80 + 2))
    fade = np.asarray(Image.fromarray(low.astype(np.float32)).resize((W, H), Image.BICUBIC))
    strength = np.clip(0.72 + 0.22 * fade, 0.38, 1.0)
    ink = ink * strength
    # 線のかすれ (細かい抜け)
    ink *= np.clip(1.0 - 0.35 * np.clip(rng.normal(0, 1, (H, W)), 0, 3), 0, 1)

    # 背景: 青みがかった紙 + 焼けムラ + シミ + 折り目
    low2 = rng.normal(0, 1, (H // 200 + 2, W // 200 + 2))
    mott = np.asarray(Image.fromarray(low2.astype(np.float32)).resize((W, H), Image.BICUBIC))
    bg = 0.80 + 0.07 * mott - 0.12 * (xx / W) ** 2
    for _ in range(6):
        sx, sy, r = rng.uniform(0, W), rng.uniform(0, H), rng.uniform(80, 260)
        bg -= 0.16 * np.exp(-(((xx - sx) ** 2 + (yy - sy) ** 2) / (2 * r * r)))
    bg -= 0.10 * np.exp(-((xx - W * 0.52) ** 2) / (2 * 6.0**2))  # 縦の折り目
    bg -= 0.08 * np.exp(-((yy - H * 0.47) ** 2) / (2 * 5.0**2))  # 横の折り目
    bg = np.clip(bg, 0.35, 1.0)
    # ごま塩ノイズ (青焼き特有の粒)
    speck = (rng.random((H, W)) < 0.0012).astype(np.float32)
    speck = np.asarray(Image.fromarray((speck * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(0.8)), float) / 255.0 * 2.2
    ink = np.clip(ink + 0.5 * speck, 0, 1)

    paper = np.stack([bg * 0.80, bg * 0.88, bg * 1.0], -1)  # 青白い紙
    line = np.array([0.16, 0.14, 0.50])  # 青紫の線
    rgb = paper * (1 - ink[..., None]) + line * ink[..., None]
    rgb += rng.normal(0, 0.022, rgb.shape)
    return Image.fromarray((np.clip(rgb, 0, 1) * 255).astype(np.uint8))


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "sample.png"
    img = draw()
    if "--clean" not in sys.argv:
        img = degrade(img)
    img.save(out)
    print("wrote", out)
