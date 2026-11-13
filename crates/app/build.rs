//! Windows 版の exe にアプリのアイコンを埋め込む。
fn main() {
    println!("cargo:rerun-if-changed=icon.ico");
    #[cfg(windows)]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icon.ico"); // リソース ID 1 (ウィンドウのアイコンにも使う)
        if let Err(e) = res.compile() {
            println!("cargo:warning=アイコンを埋め込めませんでした: {e}");
        }
    }
}
