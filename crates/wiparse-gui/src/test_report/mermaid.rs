//! Offline diagrams. No JavaScript, network fetches, or model-controlled file access.
use wiparse_core::i18n::Lang;
use egui::{ColorImage, TextureHandle};
use std::{collections::VecDeque, sync::{Arc, Mutex, OnceLock, atomic::{AtomicUsize, Ordering}}, time::{Duration, Instant}};

static RUNNING: AtomicUsize = AtomicUsize::new(0);
struct Slot;
impl Drop for Slot { fn drop(&mut self) { RUNNING.fetch_sub(1, Ordering::AcqRel); } }
struct Rendered { svg: String, png: Vec<u8>, image: ColorImage }
struct State {
    source: String, dark: bool, changed: Instant, attempted: bool,
    job: Option<crossbeam_channel::Receiver<(String, bool, Result<Rendered, String>)>>,
    texture: Option<TextureHandle>, output: Option<Rendered>, error: Option<String>, zoom: f32, expanded:bool, expanded_zoom:f32,
    export: Option<crossbeam_channel::Receiver<String>>, export_status: String,
}
impl Default for State { fn default() -> Self { Self { source: String::new(), dark: false, changed: Instant::now(), attempted: false, job: None, texture: None, output: None, error: None, zoom: 1.0, expanded:false, expanded_zoom:1.0, export: None, export_status: String::new() } } }
type Cache = Arc<Mutex<VecDeque<(egui::Id, Arc<Mutex<State>>)>>>;

fn render(source: &str, dark: bool) -> Result<Rendered, String> {
    if source.len() > 32 * 1024 || source.lines().count() > 512 { return Err("图表超过 32 KiB / 512 行限制 / Diagram too large".into()); }
    let lower = source.to_lowercase();
    // Disallow directives and active/external content, including exported SVG links.
    if ["%%{", "javascript:", "file:", "https:", "http:", "data:", "click ", "image:", "icon:", "url("].iter().any(|s| lower.contains(s)) || regex::Regex::new(r"<[a-z!/]" ).unwrap().is_match(&lower) {
        return Err("不支持 HTML、外部内容、交互链接或配置指令 / HTML, external content and directives disabled".into());
    }
    let parsed = mermaid_rs_renderer::parse_mermaid_strict(source).map_err(|e| format!("Mermaid: {e}"))?;
    if parsed.graph.nodes.len() > 128 || parsed.graph.edges.len() > 256 { return Err("图表最多 128 个节点、256 条边 / Diagram complexity limit".into()); }
    let mut theme = if dark { mermaid_rs_renderer::Theme::dark() } else { mermaid_rs_renderer::Theme::modern() };
    theme.font_family = "Microsoft YaHei, Noto Sans CJK SC, sans-serif".into();
    let config = mermaid_rs_renderer::LayoutConfig::default();
    let layout = mermaid_rs_renderer::compute_layout(&parsed.graph, &theme, &config);
    let svg = mermaid_rs_renderer::render_svg(&layout, &theme, &config);
    if svg.len() > 2 * 1024 * 1024 { return Err("SVG 超过大小限制 / SVG too large".into()); }
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    let fonts = FONTS.get_or_init(|| { let mut db = resvg::usvg::fontdb::Database::new(); db.load_system_fonts(); db.set_sans_serif_family("Microsoft YaHei"); Arc::new(db) });
    let mut options = resvg::usvg::Options::default();
    options.fontdb = fonts.clone();
    options.image_href_resolver = resvg::usvg::ImageHrefResolver { resolve_data: Box::new(|_, _, _| None), resolve_string: Box::new(|_, _| None) };
    let tree = resvg::usvg::Tree::from_str(&svg, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = (4096.0 / size.width().max(size.height())).min((2_097_152.0 / (size.width()*size.height())).sqrt()).min(2.0);
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or("无法分配图像 / Image allocation failed")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let png = pixmap.encode_png().map_err(|e| e.to_string())?;
    let image = ColorImage::from_rgba_premultiplied([width as usize, height as usize], pixmap.data());
    Ok(Rendered { svg, png, image })
}

fn fitted_size(image: egui::Vec2, area: egui::Vec2) -> egui::Vec2 {
    image * (area.x / image.x).min(area.y / image.y).min(1.0)
}

// Reserve our own canvas even when this block starts near the bottom of a chat
// scroll viewport. In fit mode only the outer conversation needs to scroll.
fn canvas(ui:&mut egui::Ui,texture:&TextureHandle,area:egui::Vec2,zoom:f32)->egui::Rect {
    let fitted=fitted_size(texture.size_vec2(),area);
    let (rect,_)=ui.allocate_exact_size(area,egui::Sense::hover());
    let mut child=ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let size=fitted*zoom;
    if size.x<=area.x+0.01 && size.y<=area.y+0.01 {
        let image=egui::Rect::from_center_size(rect.center(),size);
        child.painter().image(texture.id(),image,egui::Rect::from_min_max(egui::pos2(0.,0.),egui::pos2(1.,1.)),egui::Color32::WHITE);
    }else{
        egui::ScrollArea::both().id_salt("diagram-pan").auto_shrink([false,false]).show(&mut child,|ui|{
            ui.add(egui::Image::new(texture).fit_to_exact_size(size));
        });
    }
    rect
}

fn zoom_buttons(ui:&mut egui::Ui,zoom:&mut f32,zh:bool){
    if ui.small_button(if zh {"适应全图"} else {"Fit diagram"}).clicked(){*zoom=1.0;}
    if ui.small_button("−").clicked(){*zoom=(*zoom/1.25).max(0.25);}
    if ui.small_button("+").clicked(){*zoom=(*zoom*1.25).min(16.0);}
}

pub(super) fn show(ui: &mut egui::Ui, source: &str, lang: Lang) {
    let zh = lang == Lang::Zh;
    if source.len() > 32768 {
        ui.label(if zh {"Mermaid 源码超过 32 KiB，请拆分图表。"} else {"Mermaid exceeds 32 KiB; split the diagram."});
        if ui.button(if zh {"复制源码"} else {"Copy source"}).clicked() {ui.ctx().copy_text(source.into());}
        return;
    }
    let cache: Cache = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Cache>(egui::Id::new("mermaid-cache-v1")).clone());
    let id = ui.id();
    let state = {
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        let value = cache.iter().position(|(key,_)| *key == id).and_then(|index| cache.remove(index)).map(|(_,v)|v).unwrap_or_default();
        cache.push_back((id,value.clone()));
        while cache.len() > 8 { cache.pop_front(); }
        value
    };
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    let dark = ui.visuals().dark_mode;
    if s.source != source || s.dark != dark {
        // Reused UI IDs must not show a diagram belonging to a different message.
        if s.source != source && !source.starts_with(&s.source) { s.texture = None; s.output = None; s.expanded=false; s.zoom=1.0; }
        s.source = source.into(); s.dark = dark; s.changed = Instant::now(); s.attempted = false; s.error = None;
        s.export_status.clear();
    }
    if let Some(result) = s.job.as_ref().and_then(|rx| rx.try_recv().ok()) {
        s.job = None;
        if result.0 == s.source && result.1 == s.dark {
            match result.2 {
                Ok(mut output) => { s.texture = Some(ui.ctx().load_texture("mermaid", output.image.clone(), egui::TextureOptions::LINEAR)); output.image.pixels = Vec::new(); s.output = Some(output); s.error = None; }
                Err(e) => s.error = Some(e),
            }
        }
    }
    if !s.attempted && s.job.is_none() && s.changed.elapsed() >= Duration::from_millis(250) {
        if RUNNING.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n| (n<2).then_some(n+1)).is_ok() {
            let (tx,rx) = crossbeam_channel::bounded(1);
            let body = s.source.clone(); let ctx = ui.ctx().clone();
            let slot = Slot;
            match std::thread::Builder::new().name("mermaid-render".into()).spawn(move || {
                let _slot = slot;
                let result = std::panic::catch_unwind(|| render(&body,dark)).unwrap_or_else(|_|Err("图表渲染失败 / Renderer failed".into()));
                let _ = tx.send((body,dark,result)); ctx.request_repaint();
            }) { Ok(_) => s.job = Some(rx), Err(e) => s.error = Some(e.to_string()) }
            s.attempted = true;
        }
    }
    let pending = !s.attempted || s.job.is_some();
    if pending { ui.ctx().request_repaint_after(Duration::from_millis(100)); }
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label("Mermaid");
            zoom_buttons(ui,&mut s.zoom,zh);
            if ui.add_enabled(s.texture.is_some(),egui::Button::new(if zh {"展开大图"}else{"Expand"}).small()).clicked(){
                s.expanded=true;s.expanded_zoom=1.0;
            }
            if ui.small_button(if zh {"复制源码"} else {"Copy source"}).clicked() { ui.ctx().copy_text(source.into()); }
            for (name, ext) in [("SVG","svg"),("PNG","png")] {
                if ui.add_enabled(s.output.is_some() && !pending && s.error.is_none() && s.export.is_none(), egui::Button::new(name).small()).clicked() {
                    let output = s.output.as_ref().unwrap();
                    let bytes = if ext == "svg" { output.svg.as_bytes().to_vec() } else { output.png.clone() };
                    let (tx,rx) = crossbeam_channel::bounded(1); s.export = Some(rx);
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let status = if let Some(path) = rfd::FileDialog::new().add_filter(name,&[ext]).set_file_name(format!("diagram.{ext}")).save_file() {
                            std::fs::write(path, bytes).map(|_|"已导出 / Exported".into()).unwrap_or_else(|e|format!("导出失败 / Export failed: {e}"))
                        } else { String::new() };
                        let _ = tx.send(status); ctx.request_repaint();
                    });
                }
            }
            if pending { ui.spinner(); }
        });
        if let Some(texture) = &s.texture {
            let width=ui.available_width().max(1.0);
            let max_height=(ui.ctx().screen_rect().height()*0.65).max(1.0);
            let fit=fitted_size(texture.size_vec2(),egui::vec2(width,max_height));
            canvas(ui,texture,egui::vec2(width,fit.y.max(1.0)),s.zoom);

        }
        if let Some(error) = &s.error { ui.colored_label(ui.visuals().warn_fg_color, format!("{}\n{error}", if s.output.is_some() {if zh {"显示上一有效版本；当前源码未成功渲染"} else {"Showing last valid diagram; current source failed"}} else {if zh {"暂不能渲染，请查看源码"} else {"Unable to render; see source"}})); }
        if let Some(status) = s.export.as_ref().and_then(|rx|rx.try_recv().ok()) { s.export = None; s.export_status = status; }
        if !s.export_status.is_empty() { ui.label(&s.export_status); }
        egui::CollapsingHeader::new(if zh {"源码"} else {"Source"}).default_open(s.error.is_some() && s.output.is_none()).show(ui, |ui| { egui::ScrollArea::both().max_height(220.0).show(ui,|ui|{ui.add(egui::Label::new(egui::RichText::new(source).monospace()).selectable(true));}); });
    });
    if s.expanded {
        if let Some(texture)=s.texture.clone(){
            let screen=ui.ctx().screen_rect().size();
            let mut open=true;
            egui::Window::new(if zh {"Mermaid 流程图"}else{"Mermaid diagram"})
                .id(id.with("mermaid-viewer")).open(&mut open).collapsible(false).resizable(true)
                .default_size(screen*0.85).default_pos(ui.ctx().screen_rect().min+screen*0.065)
                .show(ui.ctx(),|ui|{
                    ui.horizontal(|ui|zoom_buttons(ui,&mut s.expanded_zoom,zh));
                    canvas(ui,&texture,ui.available_size().max(egui::vec2(1.,1.)),s.expanded_zoom);
                });
            s.expanded=open;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tall_diagram_reserves_its_height_near_end_of_chat_viewport(){
        let ctx=egui::Context::default();
        let _=ctx.run(Default::default(), |_|{});
        let texture=ctx.load_texture("tall-diagram",ColorImage::new([120,900],egui::Color32::WHITE),Default::default());
        let screen=egui::vec2(680.,600.);
        let fit=fitted_size(texture.size_vec2(),egui::vec2(650.,390.));
        assert!((fit.y-390.).abs()<0.01);
        let output=ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,screen)),..Default::default()},|ctx|{
            egui::CentralPanel::default().show(ctx,|ui|{
                egui::ScrollArea::vertical().show(ui,|ui|{
                    ui.add_space(520.);
                    let rect=canvas(ui,&texture,egui::vec2(650.,fit.y),1.);
                    assert!(rect.height()>380. && rect.bottom()>screen.y);
                    assert!(ui.cursor().min.y>=rect.bottom());
                });
            });
        });
        let image=ctx.tessellate(output.shapes,output.pixels_per_point).into_iter().find_map(|s|match s.primitive{
            egui::epaint::Primitive::Mesh(m) if m.texture_id==texture.id()=>Some(m.calc_bounds()),
            _=>None,
        }).expect("diagram image");
        assert!((image.height()-fit.y).abs()<1.,"mesh {image:?}, fit {fit:?}");
        assert!((image.width()-fit.x).abs()<1.);
    }
    #[test]
    fn chat_diagram_updates_and_failure_keeps_last_valid_texture() {
        fn tick(ctx:&egui::Context,source:&str) {
            let _=ctx.run(egui::RawInput { screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(280.0,600.0))), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx,|ui|show(ui,source,Lang::Zh));
            });
        }
        let ctx=egui::Context::default();
        let valid="flowchart LR\n A[接收] --> B[分析]";
        tick(&ctx,valid);
        let cache=ctx.data(|d|d.get_temp::<Cache>(egui::Id::new("mermaid-cache-v1")).unwrap());
        let state=cache.lock().unwrap().back().unwrap().1.clone();
        let settle=|source:&str| {
            state.lock().unwrap().changed=Instant::now()-Duration::from_secs(1);
            for _ in 0..200 {
                tick(&ctx,source);
                { let s=state.lock().unwrap(); if s.attempted && s.job.is_none() {return;} }
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!("renderer did not complete");
        };
        settle(valid);
        assert!(state.lock().unwrap().texture.is_some());
        let invalid=format!("{valid}\n<script>");tick(&ctx,&invalid);settle(&invalid);
        { let s=state.lock().unwrap(); assert!(s.error.is_some());assert!(s.texture.is_some()); }
        ctx.set_visuals(egui::Visuals::light());tick(&ctx,valid);settle(valid);
        let s=state.lock().unwrap();assert!(s.error.is_none());assert!(!s.dark);
    }
    #[test]
    fn offline_mermaid_diagrams_and_limits() {
        for source in ["flowchart LR\n A[开始] --> B[充电]\n B --> C[结束]", "sequenceDiagram\n participant A\n participant B\n A->>B: Ping\n B-->>A: ACK", "stateDiagram-v2\n [*] --> Idle\n Idle --> Charging\n Charging --> [*]"] {
            for dark in [false,true] { let out=render(source,dark).unwrap(); assert!(out.svg.contains("<svg")); assert!(out.png.starts_with(b"\x89PNG")); assert!(out.image.size[0] <= 1601 && out.image.size[1] <= 1601); }
        }
        if let Ok(folder)=std::env::var("WIPARSE_MERMAID_PROOF") {
            for dark in [false,true] {
                let output=render("flowchart LR\n A[串口接收] --> B[本地筛选]\n B --> C{异常?}\n C -->|是| D[分析与预警]\n C -->|否| E[继续监控]",dark).unwrap();
                std::fs::write(std::path::Path::new(&folder).join(if dark {"dark.png"} else {"light.png"}),output.png).unwrap();
            }
        }
        assert!(render("not a diagram",false).is_err());
        assert!(render(&"a".repeat(32769),false).is_err());
        assert!(render("flowchart LR\n A-->B\n click A \"https://example.com\"",false).is_err());
    }
}
