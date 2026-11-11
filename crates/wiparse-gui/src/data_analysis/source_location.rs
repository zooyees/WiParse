use super::DataAnalysisPanel;
use serde::Deserialize;
use std::{fs::File, io::Read, path::{Path,PathBuf}, sync::Arc};

#[derive(Clone,Deserialize)]
struct Location {
    version:u32,
    source:String,
    decoded_text_sha256:String,
    coordinate:String,
    position:u64,
}
impl Location {
    fn load(path:&Path)->Result<Self,String>{
        let mut bytes=Vec::new();
        File::open(path).and_then(|f|f.take(8193).read_to_end(&mut bytes)).map_err(|e|e.to_string())?;
        if bytes.len()>8192{return Err("报告定位文件超过大小限制".into());}
        let value:Self=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
        if value.version!=1 || value.decoded_text_sha256.len()!=64 ||
           !value.decoded_text_sha256.bytes().all(|b|b.is_ascii_hexdigit()) ||
           !matches!(value.coordinate.as_str(),"decoded_utf8_byte_offset_0based"|"source_line_1based") ||
           (value.coordinate=="source_line_1based" && value.position==0) {
            return Err("不支持的原文定位信息".into());
        }
        Ok(value)
    }
    fn excerpt(&self,text:&str)->Result<String,String>{
        if !wiparse_core::update::hex_sha256(text.as_bytes()).eq_ignore_ascii_case(&self.decoded_text_sha256) {
            return Err("文件内容与报告记录的摘要不一致，请选择采集时的源文件。".into());
        }
        let (row,byte)=if self.coordinate=="source_line_1based" {(self.position,None)}
        else{
            let offset=usize::try_from(self.position).map_err(|e|e.to_string())?;
            let prefix=text.get(..offset).ok_or("帧偏移超出文件或落在字符内部")?;
            (1+prefix.bytes().filter(|b|*b==b'\n').count() as u64,Some(offset-prefix.rfind('\n').map_or(0,|n|n+1)))
        };
        let mut out=String::new();
        let mut found=false;
        for (index,line) in text.lines().enumerate().skip(row.saturating_sub(4) as usize).take(7) {
            use std::fmt::Write;
            let active=index as u64+1==row;
            found|=active;
            let line=if active {byte.and_then(|p|line.get(p..)).unwrap_or(line)}else{line};
            let mut chars=line.chars();
            let excerpt=chars.by_ref().take(512).collect::<String>();
            let _=writeln!(out,"{} {}  {}{}",if active{"▶"}else{" "},index+1,excerpt,if chars.next().is_some(){"…"}else{""});
        }
        if !found{return Err("源记录超出文件范围".into());}
        Ok(out)
    }
}

pub(super) struct Viewer {
    location:Option<Location>,
    pending:Option<crossbeam_channel::Receiver<Result<String,String>>>,
    text:String,
    failed:bool,
}
impl Viewer {
    fn read(&mut self,ctx:&egui::Context,opened:Option<Arc<String>>,path:Option<PathBuf>){
        let Some(location)=self.location.clone() else{return;};
        let (tx,rx)=crossbeam_channel::bounded(1);
        self.text="正在核对文件并定位…".into();
        self.failed=false;
        let ctx=ctx.clone();
        match std::thread::Builder::new().name("wiparse-report-source".into()).spawn(move||{
            let result=match (opened,path){
                (Some(text),_)=>location.excerpt(&text),
                (_,Some(path))=>super::read_text_file(&path).and_then(|s|location.excerpt(&s)),
                _=>Err("请选择原始采集文件".into()),
            };
            let _=tx.send(result);
            ctx.request_repaint();
        }){
            Ok(_)=>self.pending=Some(rx),
            Err(e)=>{self.text=e.to_string();self.failed=true;}
        }
    }
    pub(super) fn show(&mut self,ctx:&egui::Context)->bool{
        if let Some(result)=self.pending.as_ref().and_then(|rx|match rx.try_recv(){
            Ok(result)=>Some(result),Err(crossbeam_channel::TryRecvError::Disconnected)=>Some(Err("原文读取中断".into())),_=>None
        }){
            self.failed=result.is_err();
            self.text=result.unwrap_or_else(|e|e);
            self.pending=None;
        }
        let mut open=true;
        egui::Window::new("报告原文定位").open(&mut open).default_width(540.).show(ctx,|ui|{
            if let Some(loc)=&self.location{ui.label(format!("{} · {} {}",loc.source,loc.coordinate,loc.position));}
            ui.add(egui::Label::new(&self.text).wrap().selectable(true));
            if self.failed && self.location.is_some() && ui.button("选择原始文件").clicked(){
                if let Some(path)=rfd::FileDialog::new().pick_file(){self.read(ctx,None,Some(path));}
            }
        });
        open
    }
}
impl DataAnalysisPanel {
    pub(crate) fn open_source_location(&mut self,path:&Path,ctx:&egui::Context){
        let mut view=Viewer{location:None,pending:None,text:String::new(),failed:false};
        match Location::load(path){
            Err(e)=>{view.text=e;view.failed=true;}
            Ok(location)=>{
                view.location=Some(location);
                view.read(ctx,self.file_text.clone(),None);
            }
        }
        self.source_view=Some(view);
    }
}
