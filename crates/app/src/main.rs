//! デスクトップ版 (Windows)。
//!
//! OS 標準の WebView2 でウィンドウを開き、実行ファイルに埋め込んだ編集画面 (web/ と wasm) を
//! 独自プロトコル経由で読み込む。ローカルサーバは立てず、ネットワークにも一切アクセスしない。
//! SVG / DXF の保存は OS の「名前を付けて保存」ダイアログで行う。
//!
//! ビルド前に `./build-web.sh` で web/pkg を生成しておくこと。

// コンソール窓を出さない
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("trace-drawing-app は Windows 専用です。他の OS では `trace-drawing serve --open` を使ってください。");
}

#[cfg(windows)]
fn main() {
    if let Err(e) = app::run() {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("図面トレース")
            .set_description(format!(
                "起動できませんでした。\n\n{e}\n\nMicrosoft Edge WebView2 ランタイムが入っていない場合は、Microsoft のサイトからインストールしてください。"
            ))
            .show();
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod app {
    use base64::Engine;
    use std::borrow::Cow;
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::platform::windows::IconExtWindows;
    use tao::window::{Icon, WindowBuilder};
    use wry::http::{header::CONTENT_TYPE, Response};
    use wry::{WebContext, WebViewBuilder};

    /// 埋め込んだ編集画面一式。
    const ASSETS: &[(&str, &[u8])] = &[
        ("index.html", include_bytes!("../../../web/index.html")),
        ("style.css", include_bytes!("../../../web/style.css")),
        ("app.js", include_bytes!("../../../web/app.js")),
        ("icon.svg", include_bytes!("../../../web/icon.svg")),
        (
            "pkg/trace_wasm.js",
            include_bytes!("../../../web/pkg/trace_wasm.js"),
        ),
        (
            "pkg/trace_wasm_bg.wasm",
            include_bytes!("../../../web/pkg/trace_wasm_bg.wasm"),
        ),
        (
            "samples/blueprint-sample.jpg",
            include_bytes!("../../../samples/blueprint-sample.jpg"),
        ),
    ];

    fn mime(path: &str) -> &'static str {
        match path.rsplit('.').next().unwrap_or("") {
            "html" => "text/html; charset=utf-8",
            "js" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "wasm" => "application/wasm",
            "jpg" => "image/jpeg",
            "png" => "image/png",
            "svg" => "image/svg+xml",
            _ => "application/octet-stream",
        }
    }

    /// 編集画面から届く保存要求。
    enum UserEvent {
        Save { name: String, data: Vec<u8> },
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let window = WindowBuilder::new()
            .with_title("図面トレース")
            // build.rs で exe に埋め込んだアイコン (リソース ID 1)。無ければ既定のアイコンのまま
            .with_window_icon(Icon::from_resource(1, None).ok())
            .with_inner_size(LogicalSize::new(1400.0, 900.0))
            .build(&event_loop)?;

        // WebView2 の作業フォルダは exe の隣ではなくユーザーのローカルデータ領域に置く
        // (exe を書き込み不可の場所に置いても動くように)
        let data_dir = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("trace-drawing")
            .join("webview");
        let mut context = WebContext::new(Some(data_dir));

        let webview = WebViewBuilder::new_with_web_context(&mut context)
            .with_custom_protocol("app".into(), |_id, request| {
                let path = request.uri().path().trim_start_matches('/');
                let path = if path.is_empty() { "index.html" } else { path };
                match ASSETS.iter().find(|(name, _)| *name == path) {
                    Some((name, body)) => Response::builder()
                        .header(CONTENT_TYPE, mime(name))
                        .body(Cow::Borrowed(*body))
                        .unwrap(),
                    None => Response::builder()
                        .status(404)
                        .header(CONTENT_TYPE, "text/plain")
                        .body(Cow::Borrowed(&b"not found"[..]))
                        .unwrap(),
                }
            })
            // 画面側の window.ipc.postMessage(JSON) を受け取る。
            // ダイアログはここ (WebView2 のコールバック内) では開かず、イベントループに回す。
            .with_ipc_handler(move |request| {
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(request.body()) else {
                    return;
                };
                if msg["cmd"] == "save" {
                    let name = msg["name"].as_str().unwrap_or("drawing").to_string();
                    let data = base64::engine::general_purpose::STANDARD
                        .decode(msg["data"].as_str().unwrap_or(""))
                        .unwrap_or_default();
                    let _ = proxy.send_event(UserEvent::Save { name, data });
                }
            })
            // 画像ファイル以外へのページ遷移 (ウィンドウに落としたファイルを開いてしまう等) を防ぐ
            .with_navigation_handler(|url| {
                url.starts_with("http://app.localhost")
                    || url.starts_with("https://app.localhost")
                    || url.starts_with("app:")
            })
            .with_url("app://localhost/index.html")
            .build(&window)?;

        event_loop.run(move |event, _, control_flow| {
            *control_flow = ControlFlow::Wait;
            match event {
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => *control_flow = ControlFlow::Exit,
                Event::UserEvent(UserEvent::Save { name, data }) => {
                    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
                    let label = match ext.as_str() {
                        "svg" => "SVG",
                        "dxf" => "DXF",
                        "json" => "作業ファイル",
                        _ => "ファイル",
                    };
                    let mut dialog = rfd::FileDialog::new()
                        .set_title("名前を付けて保存")
                        .set_file_name(&name);
                    if !ext.is_empty() {
                        dialog = dialog.add_filter(label, &[ext.as_str()]);
                    }
                    let result = match dialog.save_file() {
                        Some(path) => match std::fs::write(&path, &data) {
                            Ok(()) => format!("保存しました: {}", path.display()),
                            Err(e) => format!("保存できませんでした: {e}"),
                        },
                        None => "保存を取り消しました".to_string(),
                    };
                    // 結果を画面のステータス欄に表示する
                    let js = format!(
                        "window.__nativeSaved && window.__nativeSaved({})",
                        serde_json::Value::String(result)
                    );
                    let _ = webview.evaluate_script(&js);
                }
                _ => {}
            }
        });
    }
}
