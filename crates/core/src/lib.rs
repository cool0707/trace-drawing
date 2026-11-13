//! 青焼き・手書き図面のトレース (ラスタ → ベクタ) ライブラリ。
//!
//! 処理の流れ:
//! 1. [`binarize`] 背景のムラを取り除いて二値化
//! 2. [`rectify`]  傾き・歪みを補正
//! 3. [`skeleton`] 細線化して線の骨格をグラフに分解
//! 4. [`fit`]      直線・円弧に当てはめ (角は丸めない)
//! 5. [`post`]     同一直線の統合、破線の一本化、ハッチング・文字の分離
//! 6. [`doc`]      手修正用の編集操作、[`export`] SVG / DXF 出力

pub mod binarize;
pub mod doc;
pub mod export;
pub mod fit;
pub mod geom;
pub mod morph;
pub mod pipeline;
pub mod post;
pub mod raster;
pub mod rectify;
pub mod refine;
pub mod skeleton;
pub mod tools;

pub use doc::{Document, Editor, Layer, LineType, Shape};
pub use export::ExportOpts;
pub use pipeline::{Params, Session, Stats};
