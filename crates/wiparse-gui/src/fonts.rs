//! Install system CJK fonts so Chinese tab titles / logs render correctly.

use egui::{FontData, FontDefinitions, FontFamily};
use std::fs;
use std::sync::Arc;

pub fn install_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let candidates = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
        r"C:\Windows\Fonts\msjh.ttc",
        r"C:\Windows\Fonts\NotoSansCJKsc-Regular.otf",
    ];

    let mut loaded = None;
    for path in candidates {
        if let Ok(data) = fs::read(path) {
            loaded = Some((path, data));
            break;
        }
    }

    if let Some((path, data)) = loaded {
        let key = "wiparse_cjk".to_owned();
        fonts
            .font_data
            .insert(key.clone(), Arc::new(FontData::from_owned(data)));
        // Fallback after built-in fonts so Latin stays crisp; Chinese glyphs resolve here.
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push(key.clone());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push(key);
        tracing::info!("Loaded CJK font: {path}");
    } else {
        tracing::warn!("No CJK system font found; Chinese glyphs may show as □");
    }

    ctx.set_fonts(fonts);
}
