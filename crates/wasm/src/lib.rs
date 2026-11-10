//! ブラウザ用バインディング。状態はすべて Rust 側 (Session) に持ち、JS は描画と入力だけを担当する。

use trace_core::doc::{Layer, LineType};
use trace_core::{export, ExportOpts, Params, Session};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Tracer {
    s: Session,
    params: Params,
}

fn lt(n: u8) -> LineType {
    match n {
        1 => LineType::Dashed,
        2 => LineType::Chain,
        _ => LineType::Solid,
    }
}

#[wasm_bindgen]
impl Tracer {
    /// 画像ファイルのバイト列から作る。
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<Tracer, JsError> {
        let s = Session::from_bytes(bytes).map_err(|e| JsError::new(&e))?;
        Ok(Tracer { s, params: Params::default() })
    }

    pub fn default_params() -> String {
        serde_json::to_string(&Params::default()).unwrap()
    }

    /// 補正とトレースを実行する。戻り値は統計情報の JSON。
    pub fn process(&mut self, params_json: &str) -> Result<String, JsError> {
        let p: Params = serde_json::from_str(params_json).map_err(|e| JsError::new(&e.to_string()))?;
        self.s.process(&p);
        self.params = p;
        Ok(serde_json::to_string(&self.s.stats).unwrap())
    }

    pub fn width(&self) -> u32 {
        self.s.prepared.as_ref().map_or(0, |p| p.norm.w as u32)
    }
    pub fn height(&self) -> u32 {
        self.s.prepared.as_ref().map_or(0, |p| p.norm.h as u32)
    }

    /// 下絵の RGBA。kind: 0 = 補正済み画像, 1 = 二値画像。
    pub fn background(&self, kind: u8) -> Vec<u8> {
        let Some(p) = self.s.prepared.as_ref() else { return vec![] };
        let mut out = Vec::with_capacity(p.norm.d.len() * 4);
        if kind == 1 {
            for v in &p.bin.d {
                let c = if *v == 1 { 40 } else { 255 };
                out.extend_from_slice(&[c, c, c, 255]);
            }
        } else {
            for v in &p.norm.d {
                out.extend_from_slice(&[*v, *v, *v, 255]);
            }
        }
        out
    }

    /// 補正後の座標 → 元画像の座標 (手動 4 点補正用)。
    pub fn to_source(&self, x: f64, y: f64) -> Vec<f64> {
        self.s.to_source([x, y], &self.params).to_vec()
    }

    // ---- 文書 ----
    pub fn doc_json(&self) -> String {
        serde_json::to_string(&self.s.editor.doc).unwrap()
    }
    pub fn load_doc(&mut self, json: &str) -> Result<(), JsError> {
        let d = serde_json::from_str(json).map_err(|e| JsError::new(&e.to_string()))?;
        self.s.editor = trace_core::Editor::new(d);
        Ok(())
    }
    pub fn selection(&self) -> Vec<u32> {
        self.s.editor.sel.iter().copied().collect()
    }

    // ---- 編集操作 ----
    pub fn click(&mut self, x: f64, y: f64, tol: f64, mask: u8, additive: bool, group: bool) -> bool {
        self.s.editor.click([x, y], tol, mask, additive, group)
    }
    pub fn pick(&self, x: f64, y: f64, tol: f64, mask: u8) -> u32 {
        self.s.editor.pick([x, y], tol, mask).unwrap_or(0)
    }
    pub fn select_rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, mask: u8, additive: bool, crossing: bool) {
        self.s.editor.select_rect([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)], mask, additive, crossing)
    }
    pub fn select_all(&mut self, mask: u8) {
        self.s.editor.select_all(mask)
    }
    pub fn clear_selection(&mut self) {
        self.s.editor.clear_selection()
    }
    pub fn delete_selected(&mut self) -> u32 {
        self.s.editor.delete_selected() as u32
    }
    pub fn join_selected(&mut self) -> u32 {
        self.s.editor.join_selected() as u32
    }
    pub fn set_linetype(&mut self, n: u8) {
        self.s.editor.set_linetype(lt(n))
    }
    /// layer: 0 = 線, 1 = ハッチング, 3 = 塗り
    pub fn set_layer(&mut self, layer: u8) {
        self.s.editor.set_layer(match layer {
            1 => Layer::Hatch,
            3 => Layer::Fill,
            _ => Layer::Line,
        })
    }
    pub fn selection_to_text(&mut self) -> bool {
        self.s.editor.selection_to_text()
    }
    pub fn text_to_lines(&mut self) -> bool {
        self.s.editor.text_to_lines()
    }
    pub fn translate_selected(&mut self, dx: f32, dy: f32) {
        self.s.editor.translate_selected(dx, dy)
    }
    pub fn add_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, linetype: u8) -> u32 {
        self.s.editor.add_line([x0, y0], [x1, y1], lt(linetype))
    }
    pub fn add_circle(&mut self, cx: f64, cy: f64, r: f64) -> u32 {
        self.s.editor.add_circle([cx, cy], r)
    }
    pub fn add_text(&mut self, x: f64, y: f64, h: f64, s: &str) -> u32 {
        self.s.editor.add_text([x, y], h, s)
    }
    pub fn set_text(&mut self, id: u32, s: &str) -> bool {
        self.s.editor.set_text(id, s)
    }
    /// 戻り値 [要素 id, 頂点番号]。なければ空。
    pub fn pick_vertex(&self, x: f64, y: f64, tol: f64) -> Vec<u32> {
        self.s.editor.pick_vertex([x, y], tol).map_or(vec![], |(id, i)| vec![id, i as u32])
    }
    pub fn move_vertex(&mut self, id: u32, idx: u32, x: f64, y: f64) -> bool {
        self.s.editor.move_vertex(id, idx as usize, [x, y])
    }
    /// 戻り値 [x, y, 種類]。スナップ先がなければ空。
    pub fn snap(&self, x: f64, y: f64, tol: f64, mask: u8, exclude: u32) -> Vec<f64> {
        self.s.editor.snap([x, y], tol, mask, exclude).map_or(vec![], |(p, k)| vec![p[0], p[1], k as f64])
    }
    pub fn split_at(&mut self, x: f64, y: f64, tol: f64, mask: u8) -> bool {
        self.s.editor.split_at([x, y], tol, mask)
    }
    pub fn undo(&mut self) -> bool {
        self.s.editor.undo()
    }
    pub fn redo(&mut self) -> bool {
        self.s.editor.redo()
    }

    // ---- 出力 ----
    pub fn export_svg(&self, opts_json: &str) -> Result<String, JsError> {
        let o: ExportOpts = serde_json::from_str(opts_json).map_err(|e| JsError::new(&e.to_string()))?;
        Ok(export::to_svg(&self.s.editor.doc, &o))
    }
    pub fn export_dxf(&self, opts_json: &str) -> Result<Vec<u8>, JsError> {
        let o: ExportOpts = serde_json::from_str(opts_json).map_err(|e| JsError::new(&e.to_string()))?;
        Ok(export::to_dxf(&self.s.editor.doc, &o))
    }
}
