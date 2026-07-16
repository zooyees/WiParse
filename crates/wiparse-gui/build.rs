// Embed WiParse.ico into the Windows PE resource table (Explorer icon).
fn main() {
    let icon = std::path::Path::new("../../packaging/WiParse.ico");
    let icon_alt = std::path::Path::new("../../Icon/WiParse.ico");
    let path = if icon.is_file() {
        icon
    } else if icon_alt.is_file() {
        icon_alt
    } else {
        println!("cargo:warning=WiParse.ico not found; PE icon not embedded");
        return;
    };
    let mut res = winresource::WindowsResource::new();
    res.set_icon(path.to_str().unwrap_or(""));
    if let Err(e) = res.compile() {
        println!("cargo:warning=Failed to embed icon: {e}");
    }
    println!("cargo:rerun-if-changed={}", path.display());
}
