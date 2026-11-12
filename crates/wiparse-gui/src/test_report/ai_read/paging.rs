use super::*;
use std::{
    collections::hash_map::RandomState,
    hash::BuildHasher,
    io::{self, Seek, SeekFrom},
    sync::OnceLock,
};

const FRAGMENT: usize = 4096;
static CURSOR_KEY: OnceLock<RandomState> = OnceLock::new();

fn signature<T: std::hash::Hash>(v: T) -> u64 {
    CURSOR_KEY.get_or_init(RandomState::new).hash_one(v)
}

fn generation(path: &Path, file: fs::File) -> io::Result<u64> {
    let meta = file.metadata()?;
    Ok(signature((
        path,
        same_file::Handle::from_file(file)?,
        meta.len(),
        meta.modified()?,
    )))
}

#[derive(Clone, Copy, Hash)]
struct Position {
    offset: u64,
    line: u64,
    byte: u64,
}

impl Position {
    fn cursor(self, source: u64) -> String {
        format!(
            "{}.{}.{}.{:016x}",
            self.offset,
            self.line,
            self.byte,
            signature((source, self))
        )
    }

    fn resume(token: &str, source: u64) -> Result<Self, String> {
        let mut parts = token.split('.');
        let mut number = || {
            parts
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or("REPORT.INVALID_CURSOR")
        };
        let pos = Self {
            offset: number()?,
            line: number()?,
            byte: number()?,
        };
        if pos.cursor(source) != token {
            return Err("REPORT.SOURCE_CHANGED_OR_INVALID_CURSOR: restart this report".into());
        }
        Ok(pos)
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("AGENT.CANCELLED".into())
    } else {
        Ok(())
    }
}

/// Compatibility path for clients requesting an explicit line or within-line byte offset.
fn locate(
    reader: &mut BufReader<fs::File>,
    start: u64,
    byte: u64,
    cancel: &AtomicBool,
) -> Result<Position, String> {
    let mut p = Position {
        offset: 0,
        line: 1,
        byte: 0,
    };
    while p.line < start || p.byte < byte {
        check_cancel(cancel)?;
        let buf = reader.fill_buf().map_err(|e| e.to_string())?;
        if buf.is_empty() {
            if byte > 0 {
                return Err("REPORT.INVALID_BYTE_CURSOR".into());
            }
            break;
        }
        let newline = buf.iter().position(|&b| b == b'\n');
        let count = if p.line == start {
            let n = (byte - p.byte).min(buf.len() as u64) as usize;
            if newline.is_some_and(|i| i < n) {
                return Err("REPORT.INVALID_BYTE_CURSOR".into());
            }
            p.byte += n as u64;
            n
        } else {
            if newline.is_some() {
                p.line += 1;
            }
            newline.map_or(buf.len(), |i| i + 1)
        };
        p.offset += count as u64;
        reader.consume(count);
    }
    if byte > 0
        && reader
            .fill_buf()
            .map_err(|e| e.to_string())?
            .first()
            .is_some_and(|b| b & 0xc0 == 0x80)
    {
        return Err("REPORT.INVALID_BYTE_CURSOR".into());
    }
    Ok(p)
}

pub(super) fn read_page_at(
    path: &Path,
    token: Option<&str>,
    start: u64,
    limit: u64,
    byte: u64,
    cancel: &AtomicBool,
) -> Result<Value, String> {
    if start == 0 || !(1..=200).contains(&limit) {
        return Err("REPORT.INVALID_READ".into());
    }
    check_cancel(cancel)?;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let source = generation(path, file.try_clone().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut p = if let Some(token) = token {
        let position = Position::resume(token, source)?;
        reader
            .seek(SeekFrom::Start(position.offset))
            .map_err(|e| e.to_string())?;
        position
    } else {
        locate(&mut reader, start, byte, cancel)?
    };
    let first = p.line;
    let mut rows = Vec::new();
    let mut encoded = 0;
    let mut raw = Vec::with_capacity(FRAGMENT + 4);
    while p.offset < size && rows.len() < limit as usize {
        check_cancel(cancel)?;
        raw.clear();
        // Four bytes of lookahead distinguish a split UTF-8 scalar or CRLF at the boundary.
        (&mut reader)
            .take((FRAGMENT + 4) as u64)
            .read_until(b'\n', &mut raw)
            .map_err(|e| e.to_string())?;
        if raw.is_empty() {
            return Err("REPORT.SOURCE_CHANGED".into());
        }
        let mut text = raw.as_slice();
        if let Some(s) = text.strip_suffix(b"\n") {
            text = s.strip_suffix(b"\r").unwrap_or(s);
        }
        let mut end = text.len().min(FRAGMENT);
        // Otherwise an invalid sequence remains lossy and still consumes its original raw bytes.
        if end < text.len() {
            for start in end.saturating_sub(3)..end {
                let width = match text[start] {
                    0xc2..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf4 => 4,
                    _ => continue,
                };
                if start + width > end
                    && text
                        .get(start..start + width)
                        .is_some_and(|s| std::str::from_utf8(s).is_ok())
                {
                    end = start;
                    break;
                }
            }
        }
        let complete = end == text.len();
        let consumed = if complete { raw.len() } else { end };
        let decoded = String::from_utf8_lossy(&text[..end]);
        let row = json!({"line":p.line,"text":decoded,"start_byte":p.byte,"next_byte":if complete{None}else{Some(p.byte+end as u64)},
            "truncated":false,"line_complete":complete,"encoding_lossy":matches!(decoded,std::borrow::Cow::Owned(_))});
        encoded += row.to_string().len();
        if encoded > 32 * 1024 && !rows.is_empty() {
            break;
        }
        rows.push(row);
        p.offset += consumed as u64;
        if complete {
            p.line += 1;
            p.byte = 0;
        } else {
            p.byte += end as u64;
            break;
        }
    }
    if generation(path, fs::File::open(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        != source
    {
        return Err("REPORT.SOURCE_CHANGED: restart this report".into());
    }
    let next = (p.offset < size).then_some(p);
    Ok(
        json!({"file":path.file_name().map(|s|s.to_string_lossy()),"source_id":wiparse_core::log_analysis::source_id_for_path(path).ok(),
        "start_line":first,"lines":rows,"next_cursor":next.map(|pos|pos.cursor(source)),"next_line":next.map(|pos|pos.line),
        "next_byte":p.byte,"reached_eof":next.is_none(),"source_bytes":size,"scope":"report text page; linked images/waveforms have not been inspected"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sequential_cursor_reads_cover_every_row_of_a_large_attachment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("capture.csv");
        let mut writer = std::io::BufWriter::new(fs::File::create(&path).unwrap());
        let padding = "x".repeat(2048);
        for i in 0..16384 {
            writeln!(writer, "{i},{padding}").unwrap();
        }
        drop(writer);
        let mut cursor = None::<String>;
        let mut count = 0;
        loop {
            let page =
                read_page_at(&path, cursor.as_deref(), 1, 80, 0, &AtomicBool::new(false)).unwrap();
            for row in page["lines"].as_array().unwrap() {
                assert_eq!(row["line"], count + 1);
                assert_eq!(
                    row["text"].as_str().unwrap().split_once(',').unwrap().0,
                    count.to_string()
                );
                count += 1;
            }
            cursor = page["next_cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                assert_eq!(page["reached_eof"], true);
                break;
            }
        }
        assert_eq!(count, 16384);
    }

    #[test]
    fn cursor_pages_reach_the_tail_of_a_32_mib_line_and_reject_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.csv");
        let mut file = fs::File::create(&path).unwrap();
        // Sparse, overlong first line with a Unicode scalar crossing a fragment boundary.
        let start = 32 * 1024 * 1024;
        file.seek(SeekFrom::Start(start + 4095)).unwrap();
        file.write_all("Ω末\r\nlast\n".as_bytes()).unwrap();
        drop(file);
        let cancel = AtomicBool::new(false);
        let page = read_page_at(&path, None, 1, 2, start, &cancel).unwrap();
        let cursor = page["next_cursor"].as_str().unwrap();
        assert_eq!(page["next_byte"], start + 4095);
        let tail = read_page_at(&path, Some(cursor), 1, 2, 0, &cancel).unwrap();
        assert_eq!(tail["lines"][0]["text"], "Ω末");
        assert_eq!(tail["lines"][1]["text"], "last");
        assert_eq!(tail["reached_eof"], true);
        fs::rename(&path, dir.path().join("old")).unwrap();
        fs::copy(dir.path().join("old"), &path).unwrap();
        assert!(read_page_at(&path, Some(cursor), 1, 1, 0, &cancel).is_err());
        assert!(read_page_at(&path, Some("0.1.0.bad"), 1, 1, 0, &cancel).is_err());
    }
}

/// Cumulative counts cover every processed record. Bounded samples describe this segment.
pub(super) fn scan_table(path:&Path,rule:&Value,cancel:&AtomicBool)->Result<Value,String>{
    let file=fs::File::open(path).map_err(|e|e.to_string())?;
    let source=generation(path,file.try_clone().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let mut spec=rule.clone();
    spec.as_object_mut().ok_or("REPORT.INVALID_SCAN")?.remove("cursor");
    let key=signature((source,spec.to_string()));
    let delimiter=rule["delimiter"].as_str().unwrap_or(",").as_bytes()[0];
    let mut csv=csv::ReaderBuilder::new().flexible(true).delimiter(delimiter).from_reader(file);
    let headers=csv.headers().map_err(|e|e.to_string())?;
    let width=headers.len();
    let column=|name:&str|->Result<usize,String>{
        let mut found=headers.iter().enumerate().filter(|(_,h)|*h==name);
        let index=found.next().ok_or_else(||format!("REPORT.COLUMN_NOT_FOUND: {name}"))?.0;
        if found.next().is_some(){return Err(format!("REPORT.AMBIGUOUS_COLUMN: {name}"));}
        Ok(index)
    };
    let col=column(rule["column"].as_str().ok_or("REPORT.COLUMN_REQUIRED")?)?;
    let subtract=rule["subtract_column"].as_str().map(column).transpose()?;
    let filter=rule["where"]["column"].as_str().map(column).transpose()?;
    let expected=rule["where"]["equals"].as_str().unwrap_or("").as_bytes();
    let (mut valid,mut invalid,mut violations,mut first_violation,mut last_violation,mut excluded)=(0u64,0u64,0u64,0u64,0u64,0u64);
    if let Some(token)=rule["cursor"].as_str(){
        let mut parts=token.splitn(7,':');
        let mut count=||parts.next().and_then(|s|s.parse::<u64>().ok()).ok_or("REPORT.INVALID_SCAN_CURSOR");
        valid=count()?;invalid=count()?;violations=count()?;first_violation=count()?;last_violation=count()?;excluded=count()?;
        let p=Position::resume(parts.next().ok_or("REPORT.INVALID_SCAN_CURSOR")?,
            signature((key,valid,invalid,violations,first_violation,last_violation,excluded)))?;
        let mut pos=csv::Position::new();
        pos.set_byte(p.offset).set_line(p.line).set_record(p.byte);
        csv.seek(pos).map_err(|e|e.to_string())?;
    }
    let start=csv.position().record();
    let mut row=csv::ByteRecord::new();
    let initial=valid+invalid+excluded;
    let mut samples=Vec::new();
    let mut invalid_samples=Vec::new();
    let mut eof=false;
    let started=std::time::Instant::now();
    while !cancel.load(Ordering::Relaxed) && started.elapsed()<std::time::Duration::from_secs(20) && valid+invalid+excluded-initial<rule["max_rows"].as_u64().unwrap_or(u64::MAX){
        if !csv.read_byte_record(&mut row).map_err(|e|e.to_string())? {eof=true;break;}
        let pos=row.position().ok_or("REPORT.NO_RECORD_POSITION")?;
        if row.len()==width && filter.is_some_and(|i|row.get(i)!=Some(expected)){excluded+=1;continue;}
        let number=|i|row.get(i).and_then(|s|std::str::from_utf8(s).ok()).and_then(|s|s.trim().parse::<f64>().ok()).filter(|v|v.is_finite());
        let lhs=number(col);
        let rhs=subtract.and_then(number);
        let value=match subtract{Some(_)=>lhs.zip(rhs).map(|(a,b)|a-b).filter(|n|n.is_finite()),None=>lhs};
        let location=||json!({"record":pos.record()+1,"line":pos.line(),"byte":pos.byte()});
        match value {
            Some(v) if row.len()==width=>{
                valid+=1;
                if rule["min"].as_f64().is_some_and(|lo|v<lo) || rule["max"].as_f64().is_some_and(|hi|v>hi){
                    violations+=1;if first_violation==0 {first_violation=pos.line();}last_violation=pos.line();if samples.len()<32 {let mut sample=location();sample["value"]=json!(v);if subtract.is_some(){sample["lhs"]=json!(lhs);sample["rhs"]=json!(rhs);}samples.push(sample);}
                }
            }
            _=>{invalid+=1;if invalid_samples.len()<16{invalid_samples.push(location());}}
        }
    }
    if generation(path,fs::File::open(path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?!=source{
        return Err("REPORT.SOURCE_CHANGED: discard counts and restart".into());
    }
    let next=csv.position();
    let cursor=(!eof).then(||{
        let position=Position{offset:next.byte(),line:next.line(),byte:next.record()}.cursor(signature((key,valid,invalid,violations,first_violation,last_violation,excluded)));
        format!("{valid}:{invalid}:{violations}:{first_violation}:{last_violation}:{excluded}:{position}")
    });
    Ok(json!({"column":rule["column"],"rule":spec,"source_version":format!("{source:016x}"),"valid":valid,"invalid":invalid,"excluded":excluded,
        "violations":violations,"first_violation_line":(first_violation>0).then_some(first_violation),"last_violation_line":(last_violation>0).then_some(last_violation),"sample_scope":"current segment","violation_samples":samples,"invalid_samples":invalid_samples,"next_cursor":cursor,"reached_eof":eof,
        "coverage":{"complete":eof,"rows_scanned":valid+invalid+excluded,"segment_rows":valid+invalid+excluded-initial,"segment_start_record":start+1,"next_record":next.record()+1},
        "all_within_limits":if eof && valid>0 && violations==0 && invalid==0 {Some(true)}else if violations>0{Some(false)}else{None}}))
}

#[cfg(test)]
mod table_checks {
    use super::*;
    use std::io::Write;
    #[test]
    fn phase_filtered_difference_retains_counts_across_pages(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("table.csv");
        fs::write(&path,"phase,VIN,VBRG\nidle,12,0\ntransfer,9,8\ntransfer,10,6\ntransfer,?,7\n").unwrap();
        let cancel=AtomicBool::new(false);
        let mut rule=json!({"column":"VIN","subtract_column":"VBRG","where":{"column":"phase","equals":"transfer"},"unit":"V","min":0,"max":2,"max_rows":2});
        let mut result=scan_table(&path,&rule,&cancel).unwrap();
        while result["reached_eof"]!=true{rule["cursor"]=result["next_cursor"].clone();result=scan_table(&path,&rule,&cancel).unwrap();}
        assert_eq!(result["excluded"],1);assert_eq!(result["valid"],2);assert_eq!(result["invalid"],1);assert_eq!(result["violations"],1);assert_eq!(result["first_violation_line"],4);
    }
    #[test]
    fn entire_large_table_including_last_record_is_checked(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("table.csv");
        let mut f=std::io::BufWriter::new(fs::File::create(&path).unwrap());
        writeln!(f,"note,VIN [V]").unwrap();
        let note="a".repeat(2050);
        for n in 1..=16384 {writeln!(f,"{note},{}",match n {8192|16384=>"7",50=>"NaN",_=>"5"}).unwrap();}
        drop(f);
        let rule=json!({"column":"VIN [V]","max":6});
        let result=scan_table(&path,&rule,&AtomicBool::new(false)).unwrap();
        assert_eq!(result["coverage"]["complete"],true);assert_eq!(result["valid"],16383);
        assert_eq!(result["invalid"],1);assert_eq!(result["violations"],2);
        assert_eq!(result["violation_samples"][1]["line"],16385);
        let part=scan_table(&path,&rule,&AtomicBool::new(true)).unwrap();assert_eq!(part["coverage"]["complete"],false);
        let mut resume=rule.clone();resume["cursor"]=part["next_cursor"].clone();
        assert_eq!(scan_table(&path,&resume,&AtomicBool::new(false)).unwrap()["violations"],2);
        let mut segment=json!({"column":"VIN [V]","max":6,"max_rows":8000});
        loop {
            let scanned=scan_table(&path,&segment,&AtomicBool::new(false)).unwrap();
            if scanned["reached_eof"]==true {
                assert_eq!(scanned["coverage"]["complete"],true);assert_eq!(scanned["violations"],2);
                assert_eq!(scanned["first_violation_line"],8193);break;
            }
            segment["cursor"]=scanned["next_cursor"].clone();
        }
        fs::write(&path,"note,VIN [V]\nchanged,2").unwrap();
        assert!(scan_table(&path,&resume,&AtomicBool::new(false)).is_err());
    }
}
