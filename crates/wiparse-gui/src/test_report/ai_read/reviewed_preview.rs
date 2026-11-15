//! Immutable *display preview* paging. It is not an authoritative file snapshot.
use super::*;
use std::sync::Arc;

const FRAGMENT_BYTES:usize=2048;
const PAGE_ITEMS:usize=12;

#[derive(Clone)]
struct Fragment { locator:String, kind:&'static str, text:String, part:usize }

pub(super) struct PreviewSnapshot {
    id:uuid::Uuid,
    project:String,
    title:String,
    fragments:Vec<Fragment>,
    omitted_images:usize,
}

impl PreviewSnapshot {
    fn append(out:&mut Vec<Fragment>,locator:String,kind:&'static str,text:&str) {
        if text.is_empty() {out.push(Fragment{locator,kind,text:String::new(),part:0});return;}
        let mut start=0;let mut part=0;
        while start<text.len() {
            let mut end=(start+FRAGMENT_BYTES).min(text.len());
            while !text.is_char_boundary(end) {end-=1;}
            out.push(Fragment{locator:locator.clone(),kind,text:text[start..end].to_owned(),part});
            start=end;part+=1;
        }
    }
    fn new(project:String,title:String,preview:&PreviewKind)->Option<Self> {
        let mut fragments=Vec::new();let mut omitted_images=0;
        match preview {
            PreviewKind::Markdown(doc)=>for (i,block) in doc.blocks.iter().enumerate() {
                let locator=format!("block:{}",i+1);
                match block {
                    MdBlock::Heading{text,..}=>Self::append(&mut fragments,locator,"heading",text),
                    MdBlock::Paragraph{text}=>Self::append(&mut fragments,locator,"paragraph",text),
                    MdBlock::Bullet{text,..}|MdBlock::Ordered{text,..}|MdBlock::Task{text,..}=>Self::append(&mut fragments,locator,"list",text),
                    MdBlock::Quote{text}=>Self::append(&mut fragments,locator,"quote",text),
                    MdBlock::Code{body,..}=>Self::append(&mut fragments,locator,"code",body),
                    MdBlock::Table{rows}=>for (row,cells) in rows.iter().enumerate() {Self::append(&mut fragments,format!("{locator}:row:{}",row+1),"table_row",&cells.join(" | "));},
                    MdBlock::Image{alt,..}=>{omitted_images+=1;Self::append(&mut fragments,locator,"image_alt_only",alt);},
                    MdBlock::Rule=>Self::append(&mut fragments,locator,"rule",""),
                }
            },
            PreviewKind::Text(text)=>for (i,line) in text.lines().enumerate() {Self::append(&mut fragments,format!("preview-line:{}",i+1),"text",line);},
            _=>return None,
        }
        Some(Self{id:uuid::Uuid::new_v4(),project,title,fragments,omitted_images})
    }
    fn manifest(&self)->Value {json!({"snapshot_id":self.id.to_string(),"project":self.project,"title":self.title,
        "scope":"parsed in-memory display preview only; not original report or current external file",
        "preview_fragment_count":self.fragments.len(),"preview_byte_limit":MAX_PREVIEW_CHARS,
        "entire_file_covered":false,"external_source_verified":false,"attachments_inspected":false,"images_not_inspected":self.omitted_images})}
    fn page(&self,params:&Value)->Result<Value,String> {
        if params.get("attachment").is_some() || params.get("scan").is_some() || params.get("start_line").is_some() || params.get("start_byte").is_some() || params.get("report_id").is_some() {return Err("REPORT.PREVIEW_ONLY".into());}
        let offset=match params.get("cursor") {
            None=>0,
            Some(v)=>{let token=v.as_str().ok_or("REPORT.INVALID_CURSOR")?;let (id,offset)=token.split_once(':').ok_or("REPORT.INVALID_CURSOR")?;
                if id!=self.id.to_string(){return Err("REPORT.SOURCE_CHANGED_OR_INVALID_CURSOR".into());}
                offset.parse::<usize>().map_err(|_|"REPORT.INVALID_CURSOR")?}
        };
        if offset>self.fragments.len(){return Err("REPORT.INVALID_CURSOR".into());}
        let limit=params.get("limit").map_or(Some(PAGE_ITEMS as u64),Value::as_u64).filter(|v|(1..=PAGE_ITEMS as u64).contains(v)).ok_or("REPORT.INVALID_READ")? as usize;
        let end=offset.saturating_add(limit).min(self.fragments.len());
        Ok(json!({"ok":true,"data":{"source_id":format!("preview-{}",self.id),"source_version":self.id.to_string(),
            "provenance":self.manifest(),"page_offset":offset,"page_count":end-offset,"total_fragments":self.fragments.len(),
            "fragments":self.fragments[offset..end].iter().map(|f|json!({"locator":f.locator,"kind":f.kind,"part":f.part,"text":f.text})).collect::<Vec<_>>(),
            "next_cursor":(end<self.fragments.len()).then(||format!("{}:{}",self.id,end)),"reached_preview_end":end==self.fragments.len(),
            "coverage":{"complete":false,"reason":"only bounded display preview was captured; original file and linked artifacts unverified"}}}))
    }
}

impl TestReportPanel {
    /// Freeze only what the UI already loaded. External files can change without
    /// an in-process lease event, so this is not eligible for outbound review.
    pub(super) fn capture_untrusted_preview(&self)->Option<wiparse_core::ai::ReviewedSource<Option<Arc<PreviewSnapshot>>>> {
        if self.selected.is_none() {return None;}
        let preview=self.preview.as_ref()?;
        let source=wiparse_core::ai::ReviewedSource::capture(&self.project_source_lease,||
            PreviewSnapshot::new(self.project_id.clone(),self.preview_title.clone(),preview).map(Arc::new));
        source.filter(|source|source.value().is_some())
    }
    pub(super) fn reviewed_preview_page(&self,params:&Value)->Value {
        let Some(source)=self.capture_untrusted_preview() else {return crate::ai::agent::failure("REPORT.NO_TEXT_PREVIEW");};
        // Do not offer file-backed previews for reviewed external transmission:
        // another process may have changed the file after the UI loaded it.
        let _=(source,params);
        crate::ai::agent::failure("REPORT.REVIEW_EXTERNAL_SOURCE_UNVERIFIED: open source cannot be reliably frozen; reviewed export blocked")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_preserve_utf8_fragments_and_never_claim_file_or_image_coverage() {
        let body="芯".repeat(1800);let preview=PreviewKind::Markdown(Arc::new(parse_md(&format!("# Summary\n\n{body}\n\n![capture](image.png)"))));
        let snap=PreviewSnapshot::new("p".into(),"Report".into(),&preview).unwrap();
        let mut cursor=None::<String>;let mut joined=String::new();
        loop {let params=if let Some(ref c)=cursor {json!({"cursor":c,"limit":1})}else{json!({"limit":1})};let page=snap.page(&params).unwrap();
            assert_eq!(page["data"]["coverage"]["complete"],false);
            for fragment in page["data"]["fragments"].as_array().unwrap(){if fragment["kind"]=="paragraph" {joined.push_str(fragment["text"].as_str().unwrap());}}
            cursor=page["data"]["next_cursor"].as_str().map(str::to_owned);if cursor.is_none(){break;}}
        assert_eq!(joined,body);assert_eq!(snap.manifest()["images_not_inspected"],1);
        assert!(snap.page(&json!({"cursor":"other:0"})).is_err());
    }
    #[test]
    fn file_backed_preview_is_never_authorized_by_its_in_process_lease() {
        let mut panel=TestReportPanel::new(&AppConfig::default());
        panel.selected=Some(PathBuf::from("external.md"));panel.preview=Some(PreviewKind::Text(Arc::new("report".into())));
        let source=panel.capture_untrusted_preview().unwrap();assert!(source.is_current());
        assert_eq!(panel.reviewed_preview_page(&json!({}))["ok"],false);
        panel.commit_empty_run("next".into(),"none".into());assert!(!source.is_current());
    }
}
