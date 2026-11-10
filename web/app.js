// 図面トレースの編集画面。
// 図面データと編集操作はすべて wasm (Rust) 側にあり、ここは描画と入力の橋渡しだけを行う。
import init, { Tracer } from "./pkg/trace_wasm.js";

const $ = (id) => document.getElementById(id);
const cv = $("cv");
const ctx = cv.getContext("2d");
const stage = $("stage");

const LAYER_BIT = { line: 1, hatch: 2, text: 4, fill: 8 };
const COLORS = { solid: "#e02424", dashed: "#f08a00", chain: "#12a150", hatch: "#2aa5d8", text: "#a23bd1", fill: "rgba(120,126,138,.55)", sel: "#1f6feb" };

const state = {
  tracer: null,
  doc: null,          // 表示用の図面データ (wasm から取得した JSON)
  sel: new Set(),
  bg: null,           // 下絵 (canvas)
  view: { s: 1, x: 0, y: 0 },
  tool: "select",
  fileName: "drawing",
  rot90: 0,
  corners: null,      // 手動 4 点補正 (元画像座標)
  cornerPts: [],      // 4点補正ツールで入力中の点 (補正後座標)
  drag: null,
  pending: null,      // 線・円ツールの 1 点目
  cursor: null,       // 図面座標でのカーソル (スナップ後)
  snapKind: 0,
  dirty: false,
};

// ---------- 起動 ----------
await init();
resize();
window.addEventListener("resize", resize);

function resize() {
  const r = stage.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  cv.width = Math.max(1, Math.round(r.width * dpr));
  cv.height = Math.max(1, Math.round(r.height * dpr));
  draw();
}

function msg(t) { $("st-msg").textContent = t; }

async function busy(label, fn) {
  $("busy").hidden = false;
  $("busy").lastElementChild.textContent = label;
  await new Promise((r) => setTimeout(r, 30));
  try { return await fn(); }
  catch (e) { console.error(e); msg("エラー: " + (e.message || e)); alert("エラー: " + (e.message || e)); }
  finally { $("busy").hidden = true; }
}

// ---------- パラメータ ----------
const NUM_PARAMS = ["threshold", "trim_overshoot", "max_side", "despeckle", "min_len", "snap_hv_deg", "dash_max_gap", "hatch_max_spacing", "hatch_min_lines", "text_max_size", "fill_min_width"];
const BOOL_PARAMS = ["upscale_thin", "arcs", "dashed", "hatch", "hatch_axis", "text", "fill"];

function readParams() {
  const p = JSON.parse(Tracer.default_params());
  for (const k of NUM_PARAMS) p[k] = Number($("p-" + k).value);
  for (const k of BOOL_PARAMS) p[k] = $("p-" + k).checked;
  p.sensitivity = Number($("p-sensitivity").value) / 100;
  p.rectify = $("p-rectify").value;
  p.source = $("p-source").value;
  p.invert = Number($("p-invert").value);
  p.rot90 = state.rot90;
  p.corners = state.corners;
  return p;
}

// ---------- 読み込みとトレース ----------
async function openImage(file) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  await busy("画像を読み込み中…", () => {
    if (state.tracer) state.tracer.free();
    state.tracer = new Tracer(bytes);
    state.fileName = file.name.replace(/\.[^.]+$/, "");
    state.rot90 = 0;
    state.corners = null;
  });
  if (state.tracer) await retrace(true);
}

async function retrace(fit) {
  if (!state.tracer) return;
  if (state.dirty && !confirm("再トレースすると手修正が失われます。続けますか？")) return;
  await busy("補正・トレース中…", () => {
    const t0 = performance.now();
    const st = JSON.parse(state.tracer.process(JSON.stringify(readParams())));
    state.dirty = false;
    state.pending = null;
    state.cornerPts = [];
    if (st.binary_source) $("p-source").title = "今回は 2 値スキャンとして処理しました";
    $("btn-auto-rect").hidden = !state.corners;
    updateBackground();
    syncDoc();
    $("drop").hidden = true;
    if (fit) fitView();
    msg(`${st.width}×${st.height}px / 線幅 ${st.stroke_width.toFixed(1)}px / 傾き ${st.skew_deg.toFixed(2)}° 補正 — 線 ${st.lines}（破線・鎖線 ${st.dashed}）, 円弧 ${st.arcs}, ハッチング ${st.hatch_lines} 本, 文字 ${st.texts}, 塗り ${st.fills} — ${((performance.now() - t0) / 1000).toFixed(1)} 秒`);
  });
  draw();
}

function updateBackground() {
  const kind = Number($("bg-kind").value);
  const t = state.tracer;
  if (!t || kind < 0) { state.bg = null; return; }
  const w = t.width(), h = t.height();
  if (!w || !h) { state.bg = null; return; }
  const c = document.createElement("canvas");
  c.width = w; c.height = h;
  c.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(t.background(kind)), w, h), 0, 0);
  state.bg = c;
}

/// wasm 側の図面を取り直して再描画する (編集操作のたびに呼ぶ)。
function syncDoc() {
  state.doc = JSON.parse(state.tracer.doc_json());
  state.sel = new Set(state.tracer.selection());
  const n = state.sel.size;
  let info = "何も選択されていません";
  if (n) {
    const c = {};
    for (const e of state.doc.ents) if (state.sel.has(e.id)) c[e.layer] = (c[e.layer] || 0) + 1;
    const names = { line: "線", hatch: "ハッチング", text: "文字", fill: "塗り" };
    info = `${n} 個を選択中（` + Object.entries(c).map(([k, v]) => `${names[k]} ${v}`).join("、") + "）";
  }
  $("sel-info").textContent = info;
  const unit = state.doc.calib.unit;
  $("calib-info").textContent = `寸法 ${state.doc.dims.length} 本 / ` + (unit > 0 ? `補正済み（1 単位 = ${unit.toFixed(4)} mm）` : "未補正");
  $("wh-unit").textContent = unit > 0 ? "mm" : "px";
  draw();
}

function edited() { state.dirty = true; syncDoc(); }

// ---------- 表示 ----------
function fitView() {
  if (!state.doc) return;
  const dpr = window.devicePixelRatio || 1;
  const cw = cv.width / dpr, ch = cv.height / dpr;
  const s = Math.min(cw / state.doc.w, ch / state.doc.h) * 0.96;
  state.view = { s, x: (cw - state.doc.w * s) / 2, y: (ch - state.doc.h * s) / 2 };
  draw();
}

const toDoc = (sx, sy) => [(sx - state.view.x) / state.view.s, (sy - state.view.y) / state.view.s];
const visibleMask = () => (["line", "hatch", "text", "fill"].reduce((m, k) => m | ($("ly-" + k).checked ? LAYER_BIT[k] : 0), 0));
const pickTol = () => 7 / state.view.s;
const penRadius = () => Math.max(2, Number($("pen-r").value) || 12) / state.view.s;

function tracePath(e) {
  switch (e.t) {
    case "line": ctx.moveTo(e.a[0], e.a[1]); ctx.lineTo(e.b[0], e.b[1]); break;
    case "poly":
      e.pts.forEach((p, i) => (i ? ctx.lineTo(p[0], p[1]) : ctx.moveTo(p[0], p[1])));
      if (e.closed) ctx.closePath();
      break;
    case "circle": ctx.moveTo(e.c[0] + e.r, e.c[1]); ctx.arc(e.c[0], e.c[1], e.r, 0, Math.PI * 2); break;
    case "arc": {
      const a1 = e.a0 + e.sweep;
      ctx.moveTo(e.c[0] + e.r * Math.cos(e.a0), e.c[1] + e.r * Math.sin(e.a0));
      ctx.arc(e.c[0], e.c[1], e.r, e.a0, a1, e.sweep < 0);
      break;
    }
    case "text":
      for (const s of e.strokes) s.forEach((p, i) => (i ? ctx.lineTo(p[0], p[1]) : ctx.moveTo(p[0], p[1])));
      break;
  }
}

function draw() {
  const dpr = window.devicePixelRatio || 1;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, cv.width, cv.height);
  const doc = state.doc;
  if (!doc) return;
  const { s, x, y } = state.view;
  ctx.setTransform(dpr * s, 0, 0, dpr * s, dpr * x, dpr * y);
  const px = 1 / s; // 画面上の 1px に相当する図面上の長さ

  // 紙と下絵
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, doc.w, doc.h);
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, doc.w, doc.h);
  ctx.clip();
  if (state.bg) {
    ctx.globalAlpha = Number($("bg-alpha").value) / 100;
    ctx.imageSmoothingEnabled = s < 2;
    // 寸法による補正で図面を変形した分だけ下絵も変形して重ねる
    const m = doc.calib.m;
    ctx.save();
    ctx.transform(m[0], m[2], m[1], m[3], 0, 0);
    ctx.drawImage(state.bg, -0.5, -0.5); // 図面座標は画素の中心が整数
    ctx.restore();
    ctx.globalAlpha = 1;
  }
  ctx.restore();

  const mask = visibleMask();
  const vis = (e) => (LAYER_BIT[e.layer] & mask) !== 0;
  const stroke = (filter, color, width, dash) => {
    ctx.beginPath();
    for (const e of doc.ents) if (vis(e) && filter(e)) tracePath(e);
    ctx.strokeStyle = color;
    ctx.lineWidth = width * px;
    ctx.setLineDash(dash ? dash.map((d) => d * px) : []);
    ctx.stroke();
  };
  ctx.lineCap = "round";
  ctx.lineJoin = "miter";

  // 塗り
  ctx.beginPath();
  for (const e of doc.ents) if (vis(e) && e.layer === "fill") tracePath(e);
  ctx.fillStyle = COLORS.fill;
  ctx.fill("evenodd");

  stroke((e) => e.layer === "hatch", COLORS.hatch, 1.2);
  stroke((e) => e.layer === "text" && !e.s, COLORS.text, 1.2);
  stroke((e) => e.layer === "line" && e.lt === "solid", COLORS.solid, 1.6);
  stroke((e) => e.layer === "line" && e.lt === "dashed", COLORS.dashed, 1.8, [9, 5]);
  stroke((e) => e.layer === "line" && e.lt === "chain", COLORS.chain, 1.8, [16, 4, 3, 4]);
  ctx.setLineDash([]);

  // 文字入力済みのブロック
  ctx.fillStyle = COLORS.text;
  ctx.textBaseline = "alphabetic";
  for (const e of doc.ents) {
    if (e.t !== "text" || !vis(e)) continue;
    if (e.s) {
      ctx.font = `${Math.max(1, e.bbox[3] - e.bbox[1])}px sans-serif`;
      ctx.fillText(e.s, e.bbox[0], e.bbox[3]);
    }
    ctx.strokeStyle = "rgba(162,59,209,.35)";
    ctx.lineWidth = px;
    ctx.strokeRect(e.bbox[0], e.bbox[1], e.bbox[2] - e.bbox[0], e.bbox[3] - e.bbox[1]);
  }

  // 端点 (つながっていない箇所を見つけやすくする)
  if ($("ly-ends").checked && s > 0.6 && (mask & 1)) {
    ctx.fillStyle = "#7a1010";
    const r = 1.8 * px;
    for (const e of doc.ents) {
      if (e.t !== "line" || e.layer !== "line") continue;
      ctx.fillRect(e.a[0] - r, e.a[1] - r, 2 * r, 2 * r);
      ctx.fillRect(e.b[0] - r, e.b[1] - r, 2 * r, 2 * r);
    }
  }

  // 寸法線
  if (doc.dims.length) {
    ctx.strokeStyle = "#0a7d6c";
    ctx.fillStyle = "#0a7d6c";
    ctx.lineWidth = 1.5 * px;
    ctx.font = `${13 * px}px sans-serif`;
    ctx.textBaseline = "bottom";
    const unit = doc.calib.unit;
    for (const d of doc.dims) {
      const dx = d.b[0] - d.a[0], dy = d.b[1] - d.a[1], L = Math.hypot(dx, dy) || 1;
      const nx = (-dy / L) * 6 * px, ny = (dx / L) * 6 * px;
      ctx.beginPath();
      ctx.moveTo(d.a[0], d.a[1]); ctx.lineTo(d.b[0], d.b[1]);
      ctx.moveTo(d.a[0] - nx, d.a[1] - ny); ctx.lineTo(d.a[0] + nx, d.a[1] + ny);
      ctx.moveTo(d.b[0] - nx, d.b[1] - ny); ctx.lineTo(d.b[0] + nx, d.b[1] + ny);
      ctx.stroke();
      let label = `${+d.len.toFixed(2)} mm`;
      if (unit > 0) { const now = L * unit; if (Math.abs(now - d.len) > 0.005 * d.len) label += `（現在 ${now.toFixed(1)}）`; }
      ctx.fillText(label, (d.a[0] + d.b[0]) / 2 + 4 * px, (d.a[1] + d.b[1]) / 2 - 3 * px);
    }
  }

  // 選択
  if (state.sel.size) {
    ctx.beginPath();
    for (const e of doc.ents) if (state.sel.has(e.id)) {
      tracePath(e);
      if (e.t === "text") ctx.rect(e.bbox[0], e.bbox[1], e.bbox[2] - e.bbox[0], e.bbox[3] - e.bbox[1]);
    }
    ctx.strokeStyle = COLORS.sel;
    ctx.lineWidth = 3 * px;
    ctx.stroke();
    if (state.sel.size <= 400) {
      const r = 4 * px;
      ctx.fillStyle = "#fff";
      ctx.lineWidth = 1.5 * px;
      for (const e of doc.ents) {
        if (!state.sel.has(e.id)) continue;
        const vs = e.t === "line" ? [e.a, e.b] : e.t === "poly" ? e.pts : [];
        for (const v of vs) { ctx.fillRect(v[0] - r, v[1] - r, 2 * r, 2 * r); ctx.strokeRect(v[0] - r, v[1] - r, 2 * r, 2 * r); }
      }
    }
  }

  // 操作中のプレビュー
  const d = state.drag;
  ctx.strokeStyle = COLORS.sel;
  ctx.lineWidth = 1.5 * px;
  if (d && d.kind === "rect") {
    const crossing = d.cur[0] < d.start[0];
    ctx.setLineDash(crossing ? [5 * px, 4 * px] : []);
    ctx.fillStyle = crossing ? "rgba(18,161,80,.12)" : "rgba(31,111,235,.12)";
    ctx.strokeStyle = crossing ? "#12a150" : COLORS.sel;
    const [x0, y0] = d.start, [x1, y1] = d.cur;
    ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
    ctx.strokeRect(x0, y0, x1 - x0, y1 - y0);
    ctx.setLineDash([]);
  }
  if (d && d.kind === "vertex" && d.cur) {
    ctx.beginPath(); ctx.moveTo(d.anchor[0], d.anchor[1]); ctx.lineTo(d.cur[0], d.cur[1]); ctx.stroke();
  }
  if (state.pending && state.cursor) {
    const [ax, ay] = state.pending, [bx, by] = state.cursor;
    ctx.beginPath();
    if (state.tool === "dim") { ctx.strokeStyle = "#0a7d6c"; }
    if (state.tool === "circle") { const r = Math.hypot(bx - ax, by - ay); ctx.moveTo(ax + r, ay); ctx.arc(ax, ay, r, 0, Math.PI * 2); }
    else { ctx.moveTo(ax, ay); ctx.lineTo(bx, by); }
    ctx.stroke();
  }
  if (state.cornerPts.length) {
    ctx.fillStyle = "#12a150";
    ctx.strokeStyle = "#12a150";
    ctx.beginPath();
    state.cornerPts.forEach((p, i) => (i ? ctx.lineTo(p[0], p[1]) : ctx.moveTo(p[0], p[1])));
    if (state.cursor) ctx.lineTo(state.cursor[0], state.cursor[1]);
    ctx.stroke();
    for (const p of state.cornerPts) { ctx.beginPath(); ctx.arc(p[0], p[1], 5 * px, 0, Math.PI * 2); ctx.fill(); }
  }
  // つなぎペンの範囲
  if (state.tool === "join" && state.cursor) {
    ctx.beginPath();
    ctx.arc(state.cursor[0], state.cursor[1], penRadius(), 0, Math.PI * 2);
    ctx.fillStyle = "rgba(31,111,235,.15)";
    ctx.fill();
    ctx.strokeStyle = COLORS.sel;
    ctx.lineWidth = 1 * px;
    ctx.stroke();
  }
  // スナップ位置のマーカー
  if (state.cursor && state.snapKind && state.tool !== "corners") {
    const [cx, cy] = state.cursor, r = 6 * px;
    ctx.strokeStyle = "#12a150";
    ctx.lineWidth = 2 * px;
    ctx.beginPath();
    if (state.snapKind === 1) ctx.rect(cx - r, cy - r, 2 * r, 2 * r);
    else if (state.snapKind === 2) { ctx.moveTo(cx - r, cy - r); ctx.lineTo(cx + r, cy + r); ctx.moveTo(cx + r, cy - r); ctx.lineTo(cx - r, cy + r); }
    else ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.stroke();
  }
}

// ---------- ツール ----------
function setTool(t) {
  state.tool = t;
  state.pending = null;
  state.cornerPts = [];
  for (const b of $("tools").children) b.classList.toggle("on", b.dataset.tool === t);
  cv.style.cursor = t === "select" ? "default" : "crosshair";
  const hints = {
    select: "クリックで選択（Shift で追加、ハッチングは束ごと・Alt で 1 本ずつ）。ドラッグで範囲選択：右へ＝完全に含むもの、左へ＝触れたもの。端点をドラッグで移動。",
    line: "始点 → 終点をクリック。続けて折れ線を描けます。Shift で水平・垂直、Esc か右クリックで終了。",
    circle: "中心 → 円周上の点をクリック。",
    text: "文字を置く位置をクリック。既存の文字ブロックはダブルクリックで入力できます。",
    split: "切りたい位置で線をクリック。",
    join: "途切れた線をなぞると 1 本につながります（Shift を押しながらで破線として）。交わる 2 本だけに触れた場合は角をつなぎます。ペンの太さは左のパネルで変更。",
    trim: "はみ出した線の先（ヒゲ）を横切るようになぞると、いちばん近い交点まで切り詰めます（クリックでも可）。交点のない線は丸ごと消えます。",
    dim: "長さの分かっている 2 点を順にクリックして実寸 (mm) を入力。既存の寸法線はダブルクリックで修正・削除できます。",
    corners: "図面枠の 4 隅を順にクリックしてください（順不同）。Esc で取り消し。",
  };
  msg(hints[t]);
  draw();
}

/// スナップと Shift 拘束を適用した図面座標。
function snapped(pt, ev, exclude = 0, anchor = null) {
  let p = pt;
  state.snapKind = 0;
  if (!ev.ctrlKey && state.tracer) {
    const r = state.tracer.snap(pt[0], pt[1], 9 / state.view.s, visibleMask() & 3, exclude);
    if (r.length) { p = [r[0], r[1]]; state.snapKind = r[2]; }
  }
  if (ev.shiftKey && anchor) {
    const dx = Math.abs(p[0] - anchor[0]), dy = Math.abs(p[1] - anchor[1]);
    p = dx >= dy ? [p[0], anchor[1]] : [anchor[0], p[1]];
  }
  return p;
}

function evPos(ev) {
  const r = cv.getBoundingClientRect();
  return [ev.clientX - r.left, ev.clientY - r.top];
}

cv.addEventListener("contextmenu", (e) => e.preventDefault());

cv.addEventListener("pointerdown", (ev) => {
  if (!state.doc) return;
  cv.setPointerCapture(ev.pointerId);
  const [sx, sy] = evPos(ev);
  const pt = toDoc(sx, sy);
  // 中ボタン / 右ボタン / Space+ドラッグ で画面移動
  if (ev.button === 1 || ev.button === 2 || spaceDown) {
    if (ev.button === 2 && (state.pending || state.cornerPts.length)) { state.pending = null; state.cornerPts = []; draw(); return; }
    state.drag = { kind: "pan", sx, sy, vx: state.view.x, vy: state.view.y };
    return;
  }
  if (ev.button !== 0) return;
  const t = state.tracer;
  const mask = visibleMask();
  switch (state.tool) {
    case "select": {
      const v = t.pick_vertex(pt[0], pt[1], 8 / state.view.s);
      if (v.length && !ev.shiftKey) {
        const e = state.doc.ents.find((x) => x.id === v[0]);
        const anchor = e.t === "line" ? (v[1] === 0 ? e.b : e.a) : e.pts[Math.max(0, v[1] - 1)];
        state.drag = { kind: "vertex", id: v[0], idx: v[1], anchor, cur: null, moved: false };
        return;
      }
      state.drag = { kind: "rect", start: pt, cur: pt, sx, sy, moved: false, shift: ev.shiftKey, alt: ev.altKey };
      return;
    }
    case "line": {
      const p = snapped(pt, ev, 0, state.pending);
      if (state.pending) {
        if (Math.hypot(p[0] - state.pending[0], p[1] - state.pending[1]) > 0.5) {
          t.add_line(state.pending[0], state.pending[1], p[0], p[1], 0);
          edited();
        }
      }
      state.pending = p;
      draw();
      return;
    }
    case "circle": {
      const p = snapped(pt, ev);
      if (state.pending) {
        const r = Math.hypot(p[0] - state.pending[0], p[1] - state.pending[1]);
        if (r > 0.5) { t.add_circle(state.pending[0], state.pending[1], r); edited(); }
        state.pending = null;
      } else state.pending = p;
      draw();
      return;
    }
    case "text": {
      const id = t.pick(pt[0], pt[1], pickTol(), 4);
      const e = id && state.doc.ents.find((x) => x.id === id);
      if (e && e.t === "text") { editText(e); return; }
      const s = prompt("文字を入力");
      if (s) { t.add_text(pt[0], pt[1], Math.max(8, state.doc.sw * 8), s); edited(); }
      return;
    }
    case "split":
      if (t.split_at(pt[0], pt[1], pickTol(), mask & 3)) edited();
      return;
    case "join":
      t.clear_selection();
      state.drag = { kind: "join", last: pt, dashed: ev.shiftKey };
      t.select_stroke(pt[0], pt[1], pt[0], pt[1], penRadius(), 1);
      syncDoc();
      return;
    case "trim":
      t.begin_group();
      state.drag = { kind: "trim", last: pt, changed: t.trim_at(pt[0], pt[1], pickTol(), 1) };
      if (state.drag.changed) syncDoc();
      return;
    case "dim": {
      const p = snapped(pt, ev, 0, state.pending);
      if (state.pending) {
        const a = state.pending;
        state.pending = null;
        const px = Math.hypot(p[0] - a[0], p[1] - a[1]);
        if (px > 1) {
          const s = prompt("この 2 点間の実際の長さ [mm]（例: 図面に 7,500 とあれば 7500）");
          const len = s === null ? NaN : Number(s.replace(/[,，\s]/g, ""));
          if (len > 0) { t.add_dim(a[0], a[1], p[0], p[1], len); edited(); }
          else if (s !== null) msg("長さは正の数で入力してください");
        }
      } else state.pending = p;
      draw();
      return;
    }
    case "corners":
      state.cornerPts.push(pt);
      if (state.cornerPts.length === 4) applyCorners();
      draw();
      return;
  }
});

cv.addEventListener("pointermove", (ev) => {
  if (!state.doc) return;
  const [sx, sy] = evPos(ev);
  const pt = toDoc(sx, sy);
  $("st-pos").textContent = `${pt[0].toFixed(0)}, ${pt[1].toFixed(0)}  ×${state.view.s.toFixed(2)}`;
  const d = state.drag;
  if (d && d.kind === "pan") {
    state.view.x = d.vx + sx - d.sx;
    state.view.y = d.vy + sy - d.sy;
  } else if (d && d.kind === "rect") {
    d.cur = pt;
    if (Math.hypot(sx - d.sx, sy - d.sy) > 4) d.moved = true;
  } else if (d && d.kind === "vertex") {
    d.cur = snapped(pt, ev, d.id, d.anchor);
    d.moved = true;
    state.cursor = d.cur;
  } else if (d && d.kind === "join") {
    const hit = state.tracer.select_stroke(d.last[0], d.last[1], pt[0], pt[1], penRadius(), 1);
    d.last = pt;
    d.dashed = d.dashed || ev.shiftKey;
    state.cursor = pt;
    if (hit) { syncDoc(); return; }
  } else if (d && d.kind === "trim") {
    const hit = state.tracer.trim_stroke(d.last[0], d.last[1], pt[0], pt[1], 1);
    d.last = pt;
    state.cursor = pt;
    if (hit) { d.changed = true; syncDoc(); return; }
  } else if (state.tool === "line" || state.tool === "circle" || state.tool === "dim") {
    state.cursor = snapped(pt, ev, 0, state.pending);
  } else {
    state.cursor = pt;
    state.snapKind = 0;
  }
  draw();
});

cv.addEventListener("pointerup", (ev) => {
  const d = state.drag;
  state.drag = null;
  if (!d || !state.tracer) return;
  const t = state.tracer;
  const mask = visibleMask();
  if (d.kind === "rect") {
    if (d.moved) {
      t.select_rect(d.start[0], d.start[1], d.cur[0], d.cur[1], mask, d.shift, d.cur[0] < d.start[0]);
    } else {
      t.click(d.start[0], d.start[1], pickTol(), mask, d.shift, !d.alt);
    }
    syncDoc();
  } else if (d.kind === "join") {
    const touched = state.sel.size;
    const n = touched >= 2 ? t.join_selected_as(d.dashed) : 0;
    if (n) { edited(); msg(`なぞった線を${d.dashed ? "破線として" : ""}つなぎました（線種は 1/2/3 キーで変更、Ctrl+Z で戻せます）`); }
    else { t.clear_selection(); syncDoc(); msg(touched >= 2 ? "なぞった線は同じ直線上になく、つなげませんでした" : "つなぐには 2 本以上の線をなぞってください"); }
  } else if (d.kind === "trim") {
    t.end_group();
    if (d.changed) edited();
  } else if (d.kind === "vertex") {
    if (d.moved && d.cur) { t.move_vertex(d.id, d.idx, d.cur[0], d.cur[1]); edited(); }
    state.snapKind = 0;
    draw();
  }
});

cv.addEventListener("dblclick", (ev) => {
  if (!state.doc) return;
  const pt = toDoc(...evPos(ev));
  const dim = state.tracer.pick_dim(pt[0], pt[1], pickTol());
  if (dim && (state.tool === "select" || state.tool === "dim")) { state.pending = null; editDim(dim); return; }
  if (state.tool !== "select") return;
  const id = state.tracer.pick(pt[0], pt[1], pickTol(), 4 & visibleMask());
  const e = id && state.doc.ents.find((x) => x.id === id);
  if (e && e.t === "text") editText(e);
});

function editDim(id) {
  const d = state.doc.dims.find((x) => x.id === id);
  const s = prompt("実際の長さ [mm]（空にするとこの寸法を削除）", String(d.len));
  if (s === null) return;
  const len = Number(s.replace(/[,，\s]/g, ""));
  if (s.trim() === "") { if (state.tracer.remove_dim(id)) edited(); }
  else if (len > 0) { if (state.tracer.set_dim(id, len)) edited(); }
  else msg("長さは正の数で入力してください");
}

function editText(e) {
  const s = prompt("この文字ブロックの内容（空にするとトレースした形のまま出力）", e.s || "");
  if (s === null) return;
  if (state.tracer.set_text(e.id, s)) edited();
}

cv.addEventListener("wheel", (ev) => {
  if (!state.doc) return;
  ev.preventDefault();
  const [sx, sy] = evPos(ev);
  const f = Math.exp(-ev.deltaY * (ev.ctrlKey ? 0.01 : 0.0015));
  const ns = Math.min(60, Math.max(0.02, state.view.s * f));
  const k = ns / state.view.s;
  state.view.x = sx - (sx - state.view.x) * k;
  state.view.y = sy - (sy - state.view.y) * k;
  state.view.s = ns;
  draw();
}, { passive: false });

async function applyCorners() {
  const t = state.tracer;
  // 画面上の点 (補正後座標) を元画像の座標に戻して保存する
  state.corners = state.cornerPts.map((p) => Array.from(t.to_source(p[0], p[1])));
  state.cornerPts = [];
  setTool("select");
  await retrace(true);
}

// ---------- 選択中の操作 ----------
const ops = {
  join() {
    const n = state.tracer.join_selected();
    if (n) { edited(); msg(`${n} 箇所を接続しました`); }
    else msg("接続できる線がありません（同一直線上の線を複数、または交わる 2 本を選んでください）");
  },
  del() { if (state.tracer.delete_selected()) edited(); },
  lt(n) { if (state.sel.size) { state.tracer.set_linetype(n); edited(); } },
  layer(n) { if (state.sel.size) { state.tracer.set_layer(n); edited(); } },
  toText() { if (state.tracer.selection_to_text()) edited(); },
  unText() { if (state.tracer.text_to_lines()) edited(); },
  width() {
    const unit = state.doc.calib.unit;
    const s = prompt(`そろえる幅 [${unit > 0 ? "mm" : "px"}]`);
    if (s === null) return;
    const w = Number(s.replace(/[,，\s]/g, ""));
    if (!(w > 0)) { msg("幅は正の数で入力してください"); return; }
    try { state.tracer.equalize_width(unit > 0 ? w / unit : w); edited(); msg("2 本を平行にして幅をそろえました"); }
    catch (e) { msg(e.message || String(e)); }
  },
  grid() {
    try {
      const n = state.tracer.remove_grid(visibleMask() & 3);
      if (n) edited();
      msg(n ? `方眼の線を ${n} 本削除しました（Ctrl+Z で戻せます）` : "該当する線がありませんでした");
    } catch (e) { msg(e.message || String(e)); }
  },
  whisker() {
    const unit = state.doc.calib.unit;
    const len = Number($("wh-len").value);
    if (!(len > 0)) return;
    const n = state.tracer.trim_whiskers(unit > 0 ? len / unit : len, state.sel.size > 0);
    if (n) edited();
    msg(n ? `${n} 箇所のヒゲを切り詰めました（Ctrl+Z で戻せます）` : "該当するヒゲはありませんでした");
  },
  calib() {
    try { const m = state.tracer.calibrate(); edited(); fitView(); msg(m); }
    catch (e) { msg(e.message || String(e)); alert(e.message || String(e)); }
  },
  dimClear() {
    if (!state.doc.dims.length) return;
    state.tracer.begin_group();
    for (const d of state.doc.dims) state.tracer.remove_dim(d.id);
    state.tracer.end_group();
    edited();
  },
  undo() { if (state.tracer.undo()) edited(); },
  redo() { if (state.tracer.redo()) edited(); },
};
const guard = (fn) => () => { if (state.tracer && state.doc) fn(); };
$("op-join").onclick = guard(ops.join);
$("op-del").onclick = guard(ops.del);
$("op-solid").onclick = guard(() => ops.lt(0));
$("op-dash").onclick = guard(() => ops.lt(1));
$("op-chain").onclick = guard(() => ops.lt(2));
$("op-hatch").onclick = guard(() => ops.layer(1));
$("op-line").onclick = guard(() => ops.layer(0));
$("op-text").onclick = guard(ops.toText);
$("op-untext").onclick = guard(ops.unText);
$("op-width").onclick = guard(ops.width);
$("op-grid").onclick = guard(ops.grid);
$("op-whisker").onclick = guard(ops.whisker);
$("op-calib").onclick = guard(ops.calib);
$("op-dimclear").onclick = guard(ops.dimClear);
$("btn-undo").onclick = guard(ops.undo);
$("btn-redo").onclick = guard(ops.redo);
$("btn-fit").onclick = fitView;

let spaceDown = false;
window.addEventListener("keyup", (ev) => { if (ev.code === "Space") spaceDown = false; });
window.addEventListener("keydown", (ev) => {
  if (ev.target.matches("input, select, textarea")) return;
  if (ev.code === "Space") { spaceDown = true; ev.preventDefault(); return; }
  if (!state.tracer || !state.doc) return;
  const k = ev.key.toLowerCase();
  const mod = ev.ctrlKey || ev.metaKey;
  if (mod && k === "z") { ev.shiftKey ? ops.redo() : ops.undo(); ev.preventDefault(); return; }
  if (mod && k === "y") { ops.redo(); ev.preventDefault(); return; }
  if (mod && k === "a") { state.tracer.select_all(visibleMask()); syncDoc(); ev.preventDefault(); return; }
  if (mod) return;
  switch (k) {
    case "escape":
      if (state.pending || state.cornerPts.length) { state.pending = null; state.cornerPts = []; draw(); }
      else { state.tracer.clear_selection(); syncDoc(); }
      break;
    case "delete": case "backspace": ops.del(); ev.preventDefault(); break;
    case "j": ops.join(); break;
    case "1": ops.lt(0); break;
    case "2": ops.lt(1); break;
    case "3": ops.lt(2); break;
    case "h": ops.layer(1); break;
    case "n": ops.layer(0); break;
    case "g": ops.toText(); break;
    case "u": ops.unText(); break;
    case "f": fitView(); break;
    case "v": setTool("select"); break;
    case "l": setTool("line"); break;
    case "c": setTool("circle"); break;
    case "t": setTool("text"); break;
    case "x": setTool("split"); break;
    case "e": setTool("trim"); break;
    case "b": setTool("join"); break;
    case "d": setTool("dim"); break;
    case "w": ops.width(); break;
    case "q": ops.grid(); break;
  }
});

// ---------- UI の配線 ----------
for (const b of $("tools").children) b.onclick = () => state.doc && setTool(b.dataset.tool);
$("btn-open").onclick = () => $("file").click();
$("file").onchange = (e) => { if (e.target.files[0]) openImage(e.target.files[0]); e.target.value = ""; };
$("btn-retrace").onclick = () => retrace(false);
$("btn-auto-rect").onclick = () => { state.corners = null; retrace(true); };
$("rot-l").onclick = () => { state.rot90 = (state.rot90 + 3) % 4; state.corners = null; retrace(true); };
$("rot-r").onclick = () => { state.rot90 = (state.rot90 + 1) % 4; state.corners = null; retrace(true); };
for (const id of ["ly-line", "ly-hatch", "ly-text", "ly-fill", "ly-ends", "bg-alpha"]) $(id).oninput = draw;
$("bg-kind").onchange = () => { updateBackground(); draw(); };

stage.addEventListener("dragover", (e) => { e.preventDefault(); stage.classList.add("over"); });
stage.addEventListener("dragleave", () => stage.classList.remove("over"));
stage.addEventListener("drop", (e) => {
  e.preventDefault();
  stage.classList.remove("over");
  const f = e.dataTransfer.files[0];
  if (f) openImage(f);
});
window.addEventListener("beforeunload", (e) => { if (state.dirty) { e.preventDefault(); e.returnValue = ""; } });

// ---------- 保存と出力 ----------
function download(name, data, type) {
  const url = URL.createObjectURL(new Blob([data], { type }));
  const a = document.createElement("a");
  a.href = url; a.download = name;
  document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function exportOpts() {
  const dpi = Number($("ex-dpi").value), denom = Number($("ex-denom").value) || 1;
  // dpi は元画像の解像度。作業解像度に縮小されていればその分だけ 1 画素が大きい。
  let scale = 1;
  if (state.doc.calib.unit > 0) scale = state.doc.calib.unit;
  else if (dpi > 0) {
    const src = state.tracer.to_source(1, 0), o = state.tracer.to_source(0, 0);
    const pxPerDocPx = Math.hypot(src[0] - o[0], src[1] - o[1]) || 1;
    scale = (25.4 / dpi) * pxPerDocPx * denom;
  }
  return JSON.stringify({ line: true, hatch: $("ex-hatch").checked, text: $("ex-text").checked, fill: $("ex-fill").checked, scale, text_strokes: true });
}

$("btn-svg").onclick = guard(() => { download(state.fileName + ".svg", state.tracer.export_svg(exportOpts()), "image/svg+xml"); state.dirty = false; });
$("btn-dxf").onclick = guard(() => { download(state.fileName + ".dxf", state.tracer.export_dxf(exportOpts()), "application/dxf"); state.dirty = false; });
$("btn-save").onclick = guard(() => { download(state.fileName + ".trace.json", state.tracer.doc_json(), "application/json"); state.dirty = false; });
$("btn-load").onclick = () => $("file-doc").click();
$("file-doc").onchange = async (e) => {
  const f = e.target.files[0];
  e.target.value = "";
  if (!f) return;
  if (!state.tracer) { alert("先に元の画像を開いてから、作業ファイルを読み込んでください。"); return; }
  try { state.tracer.load_doc(await f.text()); state.dirty = false; syncDoc(); msg("作業ファイルを読み込みました"); }
  catch (err) { alert("読み込めませんでした: " + (err.message || err)); }
};

$("btn-sample").onclick = async () => {
  try {
    const r = await fetch("samples/blueprint-sample.jpg");
    if (!r.ok) throw new Error("サンプル画像が見つかりません");
    openImage(new File([await r.blob()], "blueprint-sample.jpg"));
  } catch (e) { msg(e.message || String(e)); }
};

// 自動テスト・デバッグ用
window.__trace = { state, openImage, retrace, fitView };
