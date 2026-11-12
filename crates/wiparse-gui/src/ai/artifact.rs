//! Portable, host-owned report artifacts from retained tool evidence.
use serde_json::{json, Value};
use std::{fs, path::{Path, PathBuf}, sync::{Arc,atomic::{AtomicBool,Ordering}}};
pub(crate) fn report_path(root:&Path,project:&str,id:&str)->Result<PathBuf,String>{
    if project.is_empty()||project.len()>128||!project.chars().all(|c|c.is_ascii_alphanumeric()||matches!(c,'-'|'_'))||uuid::Uuid::parse_str(id).is_err(){return Err("REPORT.INVALID_ID".into());}
    let root=root.canonicalize().map_err(|e|e.to_string())?;
    let mut path=root.clone();
    for part in ["projects",project,"ai-reports",id]{
        path.push(part);
        if path.exists()&&!path.canonicalize().map_err(|e|e.to_string())?.starts_with(&root){return Err("REPORT.PATH_OUTSIDE_PROJECT".into());}
    }
    Ok(path.join("report.md"))
}
pub(crate) fn start<T:Send+'static>(root:PathBuf,project:String,p:Value,evidence:Vec<Value>,cancel:Arc<AtomicBool>,ctx:egui::Context,wrap:fn(Value)->T)->Result<crossbeam_channel::Receiver<T>,String>{
    if evidence.is_empty()||evidence.len()>16||evidence.iter().any(|e|e["ok"]!=true||e["truncated"]==true){return Err("REPORT.EVIDENCE_MISSING_OR_TRUNCATED".into());}
    let dataset=crate::data_analysis::ai_extract::report_dataset(&ctx);
    let(tx,rx)=crossbeam_channel::bounded(1);
    std::thread::Builder::new().name("wiparse-agent-report".into()).spawn(move||{
        let value=match create_with_dataset(&root,&project,&p,&evidence,&cancel,dataset){Ok(v)=>json!({"ok":true,"data":v}),Err(e)=>json!({"ok":false,"error":e})};
        let _=tx.send(wrap(value));ctx.request_repaint();
    }).map_err(|e|e.to_string())?;Ok(rx)
}
#[cfg(test)]
fn create(root:&Path,project:&str,p:&Value,evidence:&[Value],cancel:&AtomicBool)->Result<Value,String>{
    create_with_dataset(root,project,p,evidence,cancel,None)
}
/// Resolve chart endpoints and extrema against the exact immutable text used by extraction.
fn source_links(dir:&Path,dataset:&wiparse_core::log_analysis::timeline::Dataset,text:&str,cancel:&AtomicBool)->Result<String,String>{
    use std::collections::BTreeMap;
    let mut selected=BTreeMap::<u64,Vec<String>>::new();
    if let Some(first)=dataset.rows.first(){selected.entry(first.2).or_default().push("首点".into());}
    if let Some(last)=dataset.rows.last(){selected.entry(last.2).or_default().push("末点".into());}
    for (field_index,field) in dataset.fields.iter().enumerate(){
        let mut min=None;
        let mut max=None;
        for (_,_,offset,values) in &dataset.rows{
            if cancel.load(Ordering::Relaxed){return Err("REPORT.CANCELLED".into());}
            if let Some(value)=values.get(field_index).copied().flatten().filter(|v|v.is_finite()){
                if min.is_none_or(|(v,_)|value<v){min=Some((value,*offset));}
                if max.is_none_or(|(v,_)|value>v){max=Some((value,*offset));}
            }
        }
        for (kind,point) in [("最小",min),("最大",max)]{
            if let Some((v,pos))=point{selected.entry(pos).or_default().push(format!("{} {kind} {v} {}",field.name,field.unit));}
        }
    }
    let line_coordinates=dataset.provenance["source_coordinate"]=="source_line_1based";
    let mut source_lines=text.lines().enumerate();
    let mut links=String::from("\n### 曲线原始记录\n\n");
    for (i,(position,reasons)) in selected.into_iter().enumerate(){
        let record=if line_coordinates{
            let mut found=None;
            for (n,line) in source_lines.by_ref(){
                if n%4096==0 && cancel.load(Ordering::Relaxed){return Err("REPORT.CANCELLED".into());}
                if n as u64+1==position{found=Some(line);break;}
            }
            found
        }else{
            usize::try_from(position).ok().and_then(|p|text.get(p..)).map(|tail|tail.lines().next().unwrap_or(""))
        }.ok_or("REPORT.SOURCE_POSITION_UNAVAILABLE")?;
        let name=format!("source-record-{i}.txt");
        let coordinate=if line_coordinates{"原文行号"}else{"解码后 UTF-8 字节偏移"};
        let label=format!("{coordinate} {position}");
        let mut chars=record.chars();
        let excerpt=chars.by_ref().take(1024).collect::<String>();
        let suffix=if chars.next().is_some(){"\n（此行仅展示前 1024 字符）"}else{""};
        let content=format!("{label}\n{}\n\n{excerpt}{suffix}\n",reasons.join(" · "));
        fs::write(dir.join(&name),content).map_err(|e|e.to_string())?;
        let locator=format!("source-record-{i}.wiparse-location.json");
        let location=json!({"version":1,"source":dataset.provenance["source"],"decoded_text_sha256":dataset.provenance["decoded_text_sha256"],
            "coordinate":dataset.provenance["source_coordinate"],"position":position});
        fs::write(dir.join(&locator),serde_json::to_vec(&location).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        links.push_str(&format!("- [{label}]({locator}) · [随报告保存的原文]({name})\n"));
    }
    Ok(links)
}

pub(crate) fn archive_dataset(root:&Path,project:&str,query:&str,dataset:Arc<wiparse_core::log_analysis::timeline::Dataset>,text:Arc<String>,table:Option<Arc<tempfile::NamedTempFile>>,cancel:&AtomicBool)->Result<Value,String>{
    let p=json!({"title":"数据提取记录","markdown":"","dataset_archive":true});
    let input=json!({"method":"data.extract","evidence_id":"source","result":{"data":{"query_id":query}}});
    let mut r=create_with_dataset(root,project,&p,&[input],cancel,Some((query.into(),dataset,text,table)))?;
    r.as_object_mut().unwrap().remove("relative_path");r.as_object_mut().unwrap().remove("open_tool");
    r["query_id"]=json!(query);Ok(r)
}
fn dataset_dir(root:&Path,project:&str,id:&str)->Result<PathBuf,String>{
    let project_dir=report_path(root,project,id)?.ancestors().nth(3).unwrap().to_owned();
    let base=project_dir.join("agent-datasets");
    if base.exists() && base.canonicalize().map_err(|e|e.to_string())?.parent()!=Some(project_dir.canonicalize().map_err(|e|e.to_string())?.as_path()){return Err("REPORT.ARCHIVE_OUTSIDE_PROJECT".into());}
    let dir=base.join(id);
    if dir.exists() && dir.canonicalize().map_err(|e|e.to_string())?.parent()!=Some(base.canonicalize().map_err(|e|e.to_string())?.as_path()){return Err("REPORT.ARCHIVE_OUTSIDE_PROJECT".into());}Ok(dir)
}
fn create_with_dataset(root:&Path,project:&str,p:&Value,evidence:&[Value],cancel:&AtomicBool,dataset:Option<(String,Arc<wiparse_core::log_analysis::timeline::Dataset>,Arc<String>,Option<Arc<tempfile::NamedTempFile>>)>)->Result<Value,String>{
    let conclusion=format!("{}\n{}",p["title"].as_str().unwrap_or(""),p["markdown"].as_str().unwrap_or(""));
    if wiparse_core::ai::workflow::asserted_table_pass(&conclusion) && evidence.iter().any(|e|e["method"]=="report.read"){
        let mut matched=false;
        for e in evidence.iter().filter(|e|e["method"]=="report.read"){
            let data=e["result"].get("data").unwrap_or(&e["result"]);
            if data["violations"].is_u64(){
                matched=true;
                if wiparse_core::ai::workflow::table_verdict(data)!="PASS"{return Err("REPORT.TABLE_VERDICT_CONFLICT".into());}
            }
        }
        if !matched{return Err("REPORT.COMPLETE_SCAN_REQUIRED".into());}
    }
    for e in evidence{
        let result=&e["result"];let data=result.get("data").unwrap_or(result);
        if e["method"]=="data.extract" && data["query_id"].is_string() && data["dataset_archive"]["query_id"]!=data["query_id"] && !dataset.as_ref().is_some_and(|(id,_,_,_)|Some(id.as_str())==data["query_id"].as_str()){
            return Err("REPORT.FILE_DATASET_EXPIRED: extract the current file again and cite its new evidence ID".into());
        }
    }
    let mut attached=false;
    let id=uuid::Uuid::new_v4().to_string();let path=report_path(root,project,&id)?;let archive_dir;let dir=if p["dataset_archive"]==true {archive_dir=dataset_dir(root,project,&id)?;archive_dir.as_path()}else{path.parent().unwrap()};
    if cancel.load(Ordering::Acquire){return Err("REPORT.CANCELLED".into());}
    fs::create_dir_all(dir.parent().unwrap()).map_err(|e|e.to_string())?;
    // Resolve again after creation, rejecting redirection of project parents.
    report_path(root,project,&id)?;
    let destination=dir;
    let staging=tempfile::Builder::new().prefix(".building-").tempdir_in(dir.parent().unwrap()).map_err(|e|e.to_string())?;
    let dir=staging.path();
    let mut md=format!("# {}\n\n{}\n\n## 可核验证据\n\n以下为本地工具输出；AI解释不替代原始测试判定。\n\n[工具证据 JSON](evidence.json)\n\n",p["title"].as_str().unwrap(),p["markdown"].as_str().unwrap());
    let mut curves=0;
    for (n,e) in evidence.iter().enumerate(){
        md.push_str(&format!("- `{}` · `{}`\n",e["evidence_id"].as_str().unwrap_or(""),e["method"].as_str().unwrap_or("")));
        let result=&e["result"];let data=result.get("data").unwrap_or(result);
        if data["violations"].is_u64() && e["method"]=="report.read"{
            let verdict=wiparse_core::ai::workflow::table_verdict(data);
            md.push_str(&format!("  列 {}，规则 {}，有效 {} 行，无效 {} 行，越限 {} 行，覆盖完整 {}。**{}**\n",
                data["column"],data["rule"],data["valid"],data["invalid"],data["violations"],data["coverage"]["complete"],verdict));
            for row in data["violation_samples"].as_array().into_iter().flatten(){
                md.push_str(&format!("  - 第 {} 行，实测 {}\n",row["line"],row["value"]));
            }
        }
        let windows=data.get("source_window").map(|w|("测量采样区间".to_owned(),w)).into_iter()
            .chain(data["frames"].as_array().into_iter().flatten().filter_map(|f|Some((format!("协议帧 #{}",f["index"].as_u64()?),f.get("source_window")?))));
        for (i,(label,window)) in windows.enumerate() {
            if window["first_sample"].as_u64().is_none() || window["last_sample"].as_u64().is_none() {continue;}
            let name=format!("wave-{n}-{i}.wiparse-wave.json");
            fs::write(dir.join(&name),serde_json::to_vec(window).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            md.push_str(&format!("  - [{label}]({name}) · #{} – #{}\n",window["first_sample"],window["last_sample"]));
        }
        if let Some(coverage)=data.get("coverage"){md.push_str(&format!("  覆盖：`{coverage}`\n"));}
        let archive=&data["dataset_archive"];
        if archive["report_id"].is_string() && e["method"]=="data.extract" {
            if archive["query_id"]!=data["query_id"]{return Err("REPORT.DATASET_ID_MISMATCH".into());}
            let saved=dataset_dir(root,project,archive["report_id"].as_str().unwrap())?.canonicalize().map_err(|e|e.to_string())?;
            verify_bundle(&saved,archive)?;
            let metadata:Value=serde_json::from_slice(&fs::read(saved.join("manifest.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            let mut chart_md=fs::read_to_string(saved.join("report.md")).map_err(|e|e.to_string())?;
            for (name,entry) in metadata["files"].as_object().ok_or("REPORT.INVALID_MANIFEST")?{
                if cancel.load(Ordering::Acquire){return Err("REPORT.CANCELLED".into());}
                if name=="report.md"{continue;}
                let dest=format!("extract-{n}-{name}");
                fs::copy(saved.join(name),dir.join(&dest)).map_err(|e|e.to_string())?;
                wiparse_core::update::verify_file_sha256(&dir.join(&dest),entry["sha256"].as_str().unwrap_or("")).map_err(|e|e.to_string())?;
                chart_md=chart_md.replace(&format!("]({name})"),&format!("]({dest})"));
            }
            md.push_str(&chart_md);curves+=archive["curves"].as_u64().unwrap_or(0) as usize;continue;
        }
        if let Some((id,d,source_text,full_table))=&dataset {
            if !attached && matches!(e["method"].as_str(),Some("data.extract"|"data.present")) && data["query_id"].as_str()==Some(id) {
                if let Some(hash)=d.provenance["decoded_text_sha256"].as_str(){md.push_str(&format!("\n解码后文本 SHA-256：`{hash}`\n"));}
                md.push_str(&source_links(dir,d,source_text,cancel)?);
                if let Some(table)=full_table {
                    use std::io::{Read,Write};
                    let mut input=table.reopen().map_err(|e|e.to_string())?;
                    let mut output=fs::File::create(dir.join("full-records.csv")).map_err(|e|e.to_string())?;
                    let mut bytes=[0;65536];
                    loop {
                        if cancel.load(Ordering::Relaxed){return Err("REPORT.CANCELLED".into());}
                        let n=input.read(&mut bytes).map_err(|e|e.to_string())?;
                        if n==0{break;}
                        output.write_all(&bytes[..n]).map_err(|e|e.to_string())?;
                    }
                    md.push_str("\n[全部有效横坐标的原始记录、位置及字段值](full-records.csv) · 下列曲线与逐点索引展示分桶保留的实际观测点。\n");
                }
                let x_label=if d.provenance["x_axis"]=="record_index"{"record_index"}else{"time_s"};
                {
                    use std::io::Write;
                    let mut index=std::io::BufWriter::new(fs::File::create(dir.join("source-rows.csv")).map_err(|e|e.to_string())?);
                    let coordinate=d.provenance["source_coordinate"].as_str().unwrap_or("source_record");
                    writeln!(index,"source_id,{coordinate},{x_label}").map_err(|e|e.to_string())?;
                    for (source,x,row,_) in &d.rows{
                        if cancel.load(Ordering::Relaxed){return Err("REPORT.CANCELLED".into());}
                        writeln!(index,"\"{}\",{row},{x}",source.replace('"',"\"\"")).map_err(|e|e.to_string())?;
                    }
                    index.flush().map_err(|e|e.to_string())?;
                    md.push_str("\n[逐点回溯：数据源、原记录位置与横坐标](source-rows.csv)\n");
                }
                for (i,f) in d.fields.iter().enumerate(){
                    use std::io::Write;
                    let stem=format!("dataset-{n}-{i}");
                    let mut csv=std::io::BufWriter::new(fs::File::create(dir.join(format!("{stem}.csv"))).map_err(|e|e.to_string())?);
                    writeln!(csv,"{x_label},value").map_err(|e|e.to_string())?;
                    let mut points=Vec::with_capacity(d.rows.len());
                    for (_,x,_,row) in &d.rows{
                        if cancel.load(Ordering::Relaxed){return Err("REPORT.CANCELLED".into())}
                        if let Some(y)=row[i]{writeln!(csv,"{x},{y}").map_err(|e|e.to_string())?;points.push(Some((*x,y)));}
                        else{writeln!(csv,"{x},").map_err(|e|e.to_string())?;points.push(None);}
                    }
                    csv.flush().map_err(|e|e.to_string())?;
                    chart(&points).save(dir.join(format!("{stem}.png"))).map_err(|e|e.to_string())?;
                    md.push_str(&format!("\n### {} [{}]\n\n横轴 `{x_label}`，共 {} 条源记录。\n\n![数据提取曲线]({stem}.png) · [完整点列]({stem}.csv)\n",f.name,f.unit,d.rows.len()));
                    curves+=1;
                }
                attached=true;
            }
        }
        let timeline=&data["timeline_analysis"];
        if let(Some(fields),Some(sources))=(timeline["fields"].as_array(),timeline["sources"].as_array()){
            for (s,source) in sources.iter().take(4).enumerate(){
                for (f,field) in fields.iter().take(4).enumerate(){
                    let points:Vec<_>=source["points"].as_array().into_iter().flatten().map(|v|v[0].as_f64().zip(v[2][f].as_f64())).collect();
                    if !points.iter().flatten().any(|(x,y)|x.is_finite()&&y.is_finite()){continue;}
                    let stem=format!("series-{n}-{s}-{f}");let csv=format!("time_s,value\n{}",points.iter().map(|p|p.map_or_else(||"\n".into(),|(x,y)|format!("{x},{y}\n"))).collect::<String>());
                    fs::write(dir.join(format!("{stem}.csv")),csv).map_err(|e|e.to_string())?;
                    chart(&points).save(dir.join(format!("{stem}.png"))).map_err(|e|e.to_string())?;
                    let stats=source["statistics"].get(f).cloned().unwrap_or_default();
                    md.push_str(&format!("\n### {} · {}\n\n来源 `{}`，观测时长 `{}` s。\n\n![实际本地提取曲线]({stem}.png)\n\n[曲线 CSV]({stem}.csv) · 统计：`{stats}`\n\n",field["name"].as_str().unwrap_or("series"),field["unit"].as_str().unwrap_or(""),source["source_id"].as_str().unwrap_or(""),source["observed_duration_s"]));curves+=1;
                }
            }
        }
    }
    md.push_str("\n曲线使用本地引擎返回的有界点列（可能为桶均值），不同字段独立纵轴，网格仅作趋势参考；准确数值见 CSV 和证据统计。缺失值不连接，不同来源不拼接。观测时长不自动视作完整充电周期。\n");
    fs::write(dir.join("evidence.json"),serde_json::to_vec_pretty(evidence).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if cancel.load(Ordering::Acquire){return Err("REPORT.CANCELLED: incomplete unpublished artifact directory retained".into());}
    let report_sha256=wiparse_core::update::hex_sha256(md.as_bytes());
    fs::write(dir.join("report.md"),md).map_err(|e|e.to_string())?;
    let mut files=std::collections::BTreeMap::new();
    for entry in fs::read_dir(dir).map_err(|e|e.to_string())? {
        if cancel.load(Ordering::Relaxed){return Err("REPORT.CANCELLED".into());}
        let path=entry.map_err(|e|e.to_string())?.path();
        let name=path.file_name().unwrap().to_string_lossy().into_owned();
        let digest=wiparse_core::update::hex_sha256_file(&path).map_err(|e|e.to_string())?;
        files.insert(name,json!({"bytes":path.metadata().map_err(|e|e.to_string())?.len(),"sha256":digest,"type":path.extension().and_then(|s|s.to_str())}));
    }
    let manifest=serde_json::to_vec(&json!({"version":1,"files":files})).map_err(|e|e.to_string())?;
    let manifest_sha256=wiparse_core::update::hex_sha256(&manifest);
    fs::write(dir.join("manifest.json"),manifest).map_err(|e|e.to_string())?;
    if cancel.load(Ordering::Acquire){return Err("REPORT.CANCELLED".into());}
    fs::rename(dir,destination).map_err(|e|e.to_string())?;
    Ok(json!({"report_id":id,"report_sha256":report_sha256,"manifest_sha256":manifest_sha256,"relative_path":format!("projects/{project}/ai-reports/{id}/report.md"),"curves":curves,"evidence_count":evidence.len(),"portable":true,"open_tool":"report.open"}))
}
/// Verify against the digest persisted at creation, including after moving the project.
pub(super) fn verify_bundle(dir:&Path,receipt:&Value)->Result<(),String>{
    let check=|name:&str|->Result<PathBuf,String>{
        if name.starts_with('.') || !name.bytes().all(|c|c.is_ascii_alphanumeric() || matches!(c,b'.'|b'-'|b'_')){
            return Err(format!("{name}: invalid filename"));
        }
        let path=dir.join(name).canonicalize().map_err(|e|format!("{name}: {e}"))?;
        if path.parent()!=Some(dir) {return Err(format!("{name}: outside report directory"));}
        Ok(path)
    };
    wiparse_core::update::verify_file_sha256(&check("report.md")?,receipt["report_sha256"].as_str().unwrap_or("")).map_err(|e|format!("report.md: {e}"))?;
    let expected=receipt["manifest_sha256"].as_str().ok_or("旧版报告正文已核验，附件创建时未记录摘要")?;
    let manifest=check("manifest.json")?;
    if manifest.metadata().map_err(|e|e.to_string())?.len()>1024*1024{return Err("manifest.json: invalid manifest size".into());}
    let bytes=fs::read(manifest).map_err(|e|e.to_string())?;
    if wiparse_core::update::hex_sha256(&bytes)!=expected{return Err("manifest.json: digest mismatch".into());}
    let m:Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
    let files=m["files"].as_object().ok_or("REPORT.INVALID_MANIFEST")?;
    let mut failures=Vec::new();
    for (name,meta) in files{
        if let Err(error)=check(name).and_then(|p|wiparse_core::update::verify_file_sha256(&p,meta["sha256"].as_str().unwrap_or("")).map_err(|e|format!("{name}: {e}"))){
            failures.push(error);
        }
    }
    if failures.is_empty(){Ok(())}else{Err(failures.join("、"))}
}
fn chart(points:&[Option<(f64,f64)>])->image::RgbImage{
    use image::Rgb;
    let mut img=image::RgbImage::from_pixel(960,320,Rgb([248,250,253]));
    for x in (40..940).step_by(90){for y in 20..300{img.put_pixel(x,y,Rgb([220,226,234]));}}
    for y in (20..301).step_by(56){for x in 40..940{img.put_pixel(x,y,Rgb([220,226,234]));}}
    let vals:Vec<_>=points.iter().flatten().filter(|(x,y)|x.is_finite()&&y.is_finite()).copied().collect();
    if vals.is_empty(){return img;}
    let xmin=vals.iter().map(|v|v.0).fold(f64::INFINITY,f64::min);let xmax=vals.iter().map(|v|v.0).fold(f64::NEG_INFINITY,f64::max);
    let ymin=vals.iter().map(|v|v.1).fold(f64::INFINITY,f64::min);let ymax=vals.iter().map(|v|v.1).fold(f64::NEG_INFINITY,f64::max);
    let coord=|(x,y):(f64,f64)|((40.+(x-xmin)/(xmax-xmin).max(1e-12)*899.)as i32,(299.-(y-ymin)/(ymax-ymin).max(1e-12)*279.)as i32);
    let mut prev:Option<(i32,i32)>=None;
    for p in points {
        let Some((x,y))=p.filter(|(x,y)|x.is_finite()&&y.is_finite()).map(coord)else{prev=None;continue;};
        if let Some((a,b))=prev{let steps=(x-a).abs().max((y-b).abs()).max(1);for i in 0..=steps{let px=a+(x-a)*i/steps;let py=b+(y-b)*i/steps;for dy in 0..2{img.put_pixel(px as u32,(py+dy).min(319)as u32,Rgb([44,115,210]));}}}
        img.put_pixel(x as u32,y as u32,Rgb([44,115,210]));prev=Some((x,y));
    }img
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn portable_report_contains_real_series_and_rejects_paths(){
 let root=std::env::temp_dir().join(format!("wiparse-report-{}",uuid::Uuid::new_v4()));fs::create_dir(&root).unwrap();
 let evidence=json!({"ok":true,"evidence_id":"S1","method":"log.analyze","result":{"ok":true,"data":{"coverage":{"complete":true},"timeline_analysis":{"fields":[{"name":"V","unit":"V"}],"sources":[{"source_id":"s","observed_duration_s":1,"points":[[0,1,[5]],[1,2,[4]]],"statistics":[{"min":4,"max":5}]}]}}}});
 let result=create(&root,"default",&json!({"title":"测试","markdown":"结论 S1"}),&[evidence],&AtomicBool::new(false)).unwrap();
 let path=report_path(&root,"default",result["report_id"].as_str().unwrap()).unwrap();let md=fs::read_to_string(&path).unwrap();assert!(md.contains("!"));assert!(!md.contains(&root.to_string_lossy().to_string()));
 assert_eq!(image::open(path.parent().unwrap().join("series-0-0-0.png")).unwrap().width(),960);
 assert!(fs::read_to_string(path.parent().unwrap().join("series-0-0-0.csv")).unwrap().contains("1,4"));
 assert!(report_path(&root,"../outside",result["report_id"].as_str().unwrap()).is_err());
 fs::remove_dir_all(root).unwrap();
 }
}

#[cfg(test)]
mod bundle_regression {
    use super::*;
    #[test] fn archived_series_can_be_exported_without_a_live_dataset(){
        use wiparse_core::log_analysis::timeline::{Dataset,Field};
        let root=tempfile::tempdir().unwrap();
        let dataset=Dataset{fields:vec![Field{name:"VIN".into(),unit:"V".into(),pattern:String::new(),scale:1.}],
            rows:vec![("src".into(),0.,1,vec![Some(5.)])],omitted:0,provenance:json!({"source_coordinate":"source_line_1based"})};
        let cancel=AtomicBool::new(false);
        let receipt=archive_dataset(root.path(),"demo","query",Arc::new(dataset),Arc::new("VIN=5".into()),None,&cancel).unwrap();
        let ev=json!({"method":"data.extract","evidence_id":"E1","ok":true,"result":{"data":{"query_id":"query","dataset_archive":receipt}}});
        let report=create(root.path(),"demo",&json!({"title":"VIN","markdown":"scope"}),&[ev],&cancel).unwrap();
        assert_eq!(report["curves"],1);
        let md=report_path(root.path(),"demo",report["report_id"].as_str().unwrap()).unwrap();
        assert!(fs::read_to_string(md).unwrap().contains("extract-0-dataset-0-0.png"));
    }
    #[test] fn portable_bundle_tracks_attachment_replacement_and_rename(){
        let root=tempfile::tempdir().unwrap();
        let receipt=create(root.path(),"demo",&json!({"title":"Test","markdown":"Results"}),&[json!({"ok":true,"method":"calc.evaluate","result":{"ok":true,"data":{"value":3}}})],&AtomicBool::new(false)).unwrap();
        let path=report_path(root.path(),"demo",receipt["report_id"].as_str().unwrap()).unwrap();
        let moved=root.path().join("moved");fs::rename(path.parent().unwrap(),&moved).unwrap();let dir=moved.canonicalize().unwrap();
        verify_bundle(&dir,&receipt).unwrap();
        fs::rename(dir.join("evidence.json"),dir.join("renamed.json")).unwrap();
        assert!(verify_bundle(&dir,&receipt).unwrap_err().contains("evidence.json"));
        fs::write(dir.join("evidence.json"),"[]").unwrap();assert!(verify_bundle(&dir,&receipt).is_err());
        fs::write(dir.join("report.md"),"changed").unwrap();assert!(verify_bundle(&dir,&receipt).unwrap_err().contains("report.md"));
    }
}
