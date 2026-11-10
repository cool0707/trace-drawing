#!/bin/sh
# ブラウザ版 (wasm) をビルドして web/pkg に出力する。
#   必要なもの: rustup target add wasm32-unknown-unknown
#               cargo install wasm-bindgen-cli   (Cargo.lock の wasm-bindgen と同じバージョン)
set -e
cd "$(dirname "$0")"
cargo build --release -p trace-wasm --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir web/pkg \
  target/wasm32-unknown-unknown/release/trace_wasm.wasm
echo "web/pkg を更新しました。'cargo run --release -p trace-drawing -- serve' で起動できます。"
