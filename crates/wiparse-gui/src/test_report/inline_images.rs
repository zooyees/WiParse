//! Upgrade legacy report image links for display without rewriting user files.
use super::*;
use std::collections::HashSet;

fn image_key(src: &str) -> String {
    let key = lexical_normalize(Path::new(&decode_md_src(src)))
        .to_string_lossy()
        .replace('\\', "/");
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

pub(super) fn local_image(src: &str) -> bool {
    let decoded = decode_md_src(src);
    looks_like_local_file(&decoded) && is_image_file(Path::new(&decoded))
}

fn extract_images(text: &mut String, images: &mut Vec<(String, String)>) {
    // Some older report tables contain a bare path instead of Markdown.
    if !text.contains("](") && local_image(text) {
        let src = text.trim().to_owned();
        let decoded = decode_md_src(&src);
        let alt = Path::new(&decoded)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        *text = alt.clone();
        images.push((alt, src));
        return;
    }
    let chars: Vec<_> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            out.extend(&chars[i..i + 2]);
            i += 2;
            continue;
        }
        if chars[i] == '`' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '`' {
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            out.extend(&chars[start..i]);
            continue;
        }
        let start = if chars[i] == '!' && chars.get(i + 1) == Some(&'[') {
            i + 1
        } else {
            i
        };
        if chars.get(start) == Some(&'[') {
            if let Some((label, src, end)) = parse_md_link(&chars, start) {
                if local_image(&src) {
                    let alt = if label.is_empty() {
                        "波形截图".to_owned()
                    } else {
                        label
                    };
                    out.push_str(&alt);
                    images.push((alt, src));
                } else {
                    out.extend(&chars[i..end]);
                }
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    *text = out;
}

pub(super) fn parse_report_md(source: &str) -> MdDoc {
    let parsed = parse_md(source);
    // Prefer an explicit ![] already supplied by the report; don't show twice.
    let mut shown: HashSet<_> = parsed
        .blocks
        .iter()
        .filter_map(|block| match block {
            MdBlock::Image { src, .. } => Some(image_key(src)),
            _ => None,
        })
        .collect();
    let mut blocks = Vec::with_capacity(parsed.blocks.len());
    for mut block in parsed.blocks {
        let mut images = Vec::new();
        match &mut block {
            MdBlock::Paragraph { text }
            | MdBlock::Bullet { text, .. }
            | MdBlock::Ordered { text, .. }
            | MdBlock::Task { text, .. }
            | MdBlock::Quote { text } => extract_images(text, &mut images),
            MdBlock::Table { rows } => {
                for row in rows {
                    for cell in row {
                        extract_images(cell, &mut images);
                    }
                }
            }
            _ => {}
        }
        blocks.push(block);
        // A table remains compact; its screenshots follow it at report width.
        for (alt, src) in images {
            if shown.insert(image_key(&src)) {
                blocks.push(MdBlock::Image { alt, src });
            }
        }
    }
    MdDoc { blocks }
}

#[cfg(test)]
mod tests;
