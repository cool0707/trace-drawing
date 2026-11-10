#!/usr/bin/env python3
"""トレース結果 (JSON) をレイヤごとに色分けした PNG にする (確認用)。
  python3 tools/preview.py doc.json out.png [下絵.png]
線=黒 / 破線=橙 / 一点鎖線=緑 / 円・円弧=赤 / ハッチング=水色 / 文字=紫 / 塗り=灰
"""
import json, math, sys
from PIL import Image, ImageDraw

doc = json.load(open(sys.argv[1]))
W, H = int(doc["w"]), int(doc["h"])
if len(sys.argv) > 3:
    img = Image.open(sys.argv[3]).convert("RGB").resize((W, H))
    img = Image.blend(img, Image.new("RGB", (W, H), "white"), 0.6)
else:
    img = Image.new("RGB", (W, H), "white")
d = ImageDraw.Draw(img)
for e in doc["ents"]:
    t, layer, lt = e["t"], e["layer"], e["lt"]
    col = {"hatch": (90, 190, 240), "text": (170, 60, 200), "fill": (150, 150, 150)}.get(layer)
    if col is None:
        col = {"solid": (0, 0, 0), "dashed": (240, 130, 0), "chain": (0, 160, 60)}[lt]
    if t == "line":
        d.line([tuple(e["a"]), tuple(e["b"])], fill=col, width=3 if layer == "line" else 2)
        if layer == "line":
            for p in (e["a"], e["b"]):
                d.rectangle([p[0] - 3, p[1] - 3, p[0] + 3, p[1] + 3], outline=(0, 90, 255))
    elif t == "poly":
        pts = [tuple(p) for p in e["pts"]]
        if e["closed"]:
            pts.append(pts[0])
        if layer == "fill":
            d.polygon(pts, fill=col)
        else:
            d.line(pts, fill=(120, 80, 0), width=3)
    elif t == "circle":
        c, r = e["c"], e["r"]
        d.ellipse([c[0] - r, c[1] - r, c[0] + r, c[1] + r], outline=(230, 0, 0), width=3)
    elif t == "arc":
        c, r, a0, sw = e["c"], e["r"], e["a0"], e["sweep"]
        n = max(8, int(abs(sw) * r / 4))
        pts = [(c[0] + r * math.cos(a0 + sw * i / n), c[1] + r * math.sin(a0 + sw * i / n)) for i in range(n + 1)]
        d.line(pts, fill=(230, 0, 0), width=3)
    elif t == "text":
        for s in e["strokes"]:
            if len(s) >= 2:
                d.line([tuple(p) for p in s], fill=col, width=2)
        d.rectangle(e["bbox"], outline=(220, 170, 240))
img.save(sys.argv[2])
