//! コマンドライン版: 画像を一括でトレースして SVG / DXF を書き出す。編集用の Web UI を配信することもできる。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use trace_core::{export, raster, ExportOpts, Params, Session};

const USAGE: &str = "\
使い方:
  trace-drawing <入力画像> -o <出力.svg|.dxf|.json> [-o ...] [オプション]
  trace-drawing serve [--port 8080] [--dir web]     編集用 Web UI をローカルで配信

オプション:
  --params <file.json>   パラメータ JSON (Web UI の「設定を保存」と同じ形式)
  --set <key=value>      パラメータを個別に上書き (例: --set sensitivity=0.7 --set hatch=false)
  --dpi <n>              スキャン解像度。指定すると出力単位が mm になる
  --scale <n>            縮尺の分母 (1/100 なら 100)。--dpi と併用して実寸で出力
  --with-hatch           ハッチングも出力する (既定では出力しない)
  --no-text              文字レイヤを出力しない
  --debug <dir>          補正後の画像・二値画像を PNG で書き出す
  --print-params         既定のパラメータ JSON を表示
";

fn main() {
    if let Err(e) = run() {
        eprintln!("エラー: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print!("{USAGE}");
        return Ok(());
    }
    if args[0] == "serve" {
        return serve(&args[1..]);
    }
    let mut input: Option<String> = None;
    let mut outputs: Vec<String> = Vec::new();
    let mut pv = serde_json::to_value(Params::default()).unwrap();
    let mut eo = ExportOpts::default();
    let (mut dpi, mut denom) = (0.0f64, 1.0f64);
    let mut debug: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().cloned().ok_or(format!("{name} には値が必要です"));
        match a.as_str() {
            "-o" | "--output" => outputs.push(val("-o")?),
            "--params" => {
                let s = std::fs::read_to_string(val("--params")?).map_err(|e| e.to_string())?;
                let v: serde_json::Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
                merge(&mut pv, &v);
            }
            "--set" => {
                let kv = val("--set")?;
                let (k, v) = kv
                    .split_once('=')
                    .ok_or("--set は key=value の形式で指定してください")?;
                let v = serde_json::from_str(v).unwrap_or(serde_json::Value::String(v.to_string()));
                if pv.get(k).is_none() {
                    return Err(format!("不明なパラメータ: {k}"));
                }
                pv[k] = v;
            }
            "--dpi" => dpi = val("--dpi")?.parse().map_err(|_| "--dpi の値が不正です")?,
            "--scale" => {
                denom = val("--scale")?
                    .parse()
                    .map_err(|_| "--scale の値が不正です")?
            }
            "--with-hatch" => eo.hatch = true,
            "--no-text" => eo.text = false,
            "--debug" => debug = Some(val("--debug")?),
            "--print-params" => {
                println!("{}", serde_json::to_string_pretty(&pv).unwrap());
                return Ok(());
            }
            s if s.starts_with('-') => return Err(format!("不明なオプション: {s}\n\n{USAGE}")),
            s => input = Some(s.to_string()),
        }
    }
    let params: Params =
        serde_json::from_value(pv).map_err(|e| format!("パラメータが不正です: {e}"))?;
    let input = input.ok_or("入力画像を指定してください")?;
    let bytes = std::fs::read(&input).map_err(|e| format!("{input}: {e}"))?;
    let mut sess = Session::from_bytes(&bytes)?;
    let t = std::time::Instant::now();
    let st = sess.process(&params).clone();
    eprintln!(
        "{}x{} 線幅 {:.1}px 傾き {:.2}° | 線 {} (うち破線 {}) 円弧 {} ハッチング {} 本/{} 群 文字 {} 塗り {} | {:.1}s",
        st.width,
        st.height,
        st.stroke_width,
        st.skew_deg,
        st.lines,
        st.dashed,
        st.arcs,
        st.hatch_lines,
        st.hatch_groups,
        st.texts,
        st.fills,
        t.elapsed().as_secs_f64()
    );
    if dpi > 0.0 {
        // 作業解像度に縮小した分も考慮して 1 画素あたりの mm を求める
        let long = sess.src.w.max(sess.src.h) as f64;
        let shrink = if params.max_side > 0 && long > params.max_side as f64 {
            long / params.max_side as f64
        } else {
            1.0
        };
        eo.scale = 25.4 / dpi * shrink * denom;
    }
    if let Some(dir) = debug {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let prep = sess.prepared.as_ref().unwrap();
        std::fs::write(
            Path::new(&dir).join("rectified.png"),
            raster::encode_png(&prep.norm),
        )
        .map_err(|e| e.to_string())?;
        let mut b = prep.bin.clone();
        b.d.iter_mut()
            .for_each(|v| *v = if *v == 1 { 0 } else { 255 });
        std::fs::write(Path::new(&dir).join("binary.png"), raster::encode_png(&b))
            .map_err(|e| e.to_string())?;
    }
    if outputs.is_empty() {
        outputs.push(
            Path::new(&input)
                .with_extension("svg")
                .to_string_lossy()
                .into_owned(),
        );
    }
    for out in outputs {
        let ext = Path::new(&out)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let data = match ext.as_str() {
            "svg" => export::to_svg(&sess.editor.doc, &eo).into_bytes(),
            "dxf" => export::to_dxf(&sess.editor.doc, &eo),
            "json" => serde_json::to_vec(&sess.editor.doc).unwrap(),
            _ => {
                return Err(format!(
                    "{out}: 拡張子は .svg / .dxf / .json のいずれかにしてください"
                ))
            }
        };
        std::fs::write(&out, data).map_err(|e| format!("{out}: {e}"))?;
        eprintln!("書き出し: {out}");
    }
    Ok(())
}

fn merge(dst: &mut serde_json::Value, src: &serde_json::Value) {
    if let (Some(d), Some(s)) = (dst.as_object_mut(), src.as_object()) {
        for (k, v) in s {
            d.insert(k.clone(), v.clone());
        }
    }
}

/// 依存なしの最小限の静的ファイルサーバ (ローカル専用)。
fn serve(args: &[String]) -> Result<(), String> {
    let mut port = 8080u16;
    let mut dir = PathBuf::from("web");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => {
                port = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--port の値が不正です")?
            }
            "--dir" => dir = PathBuf::from(it.next().ok_or("--dir には値が必要です")?),
            s => return Err(format!("不明なオプション: {s}")),
        }
    }
    let root = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    if !root.join("pkg").exists() {
        eprintln!(
            "注意: {}/pkg がありません。先に ./build-web.sh を実行してください。",
            root.display()
        );
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    eprintln!("http://127.0.0.1:{port}/ で配信中 (Ctrl+C で終了)");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let root = root.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req.split_whitespace().nth(1).unwrap_or("/");
            let path = path
                .split(['?', '#'])
                .next()
                .unwrap_or("/")
                .trim_start_matches('/');
            let rel = if path.is_empty() { "index.html" } else { path };
            let file = root
                .join(rel)
                .canonicalize()
                .ok()
                .filter(|p| p.starts_with(&root) && p.is_file());
            let resp = match file.and_then(|f| std::fs::read(&f).ok().map(|b| (f, b))) {
                Some((f, body)) => {
                    let mime = match f.extension().and_then(|e| e.to_str()).unwrap_or("") {
                        "html" => "text/html; charset=utf-8",
                        "js" => "text/javascript; charset=utf-8",
                        "css" => "text/css; charset=utf-8",
                        "wasm" => "application/wasm",
                        "svg" => "image/svg+xml",
                        "png" => "image/png",
                        "json" => "application/json",
                        _ => "application/octet-stream",
                    };
                    let mut r = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                    r.extend_from_slice(&body);
                    r
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found".to_vec(),
            };
            let _ = stream.write_all(&resp);
        });
    }
    Ok(())
}
