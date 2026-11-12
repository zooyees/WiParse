use serde_json::{json, Value};
use std::{io::Read, path::Path, sync::atomic::{AtomicBool,Ordering}};

pub(super) fn inspect(path:&Path,cancel:&AtomicBool)->Result<Value,String>{
    let mut bytes=Vec::new();
    std::fs::File::open(path).map_err(|e|e.to_string())?.take(16*1024*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    if bytes.len()>16*1024*1024 {return Err("REPORT.IMAGE_BYTE_LIMIT".into());}
    let reader=||image::ImageReader::new(std::io::Cursor::new(bytes.as_slice())).with_guessed_format().map_err(|e|e.to_string());
    let (w,h)=reader()?.into_dimensions().map_err(|e|e.to_string())?;
    if u64::from(w)*u64::from(h)>12_000_000 || cancel.load(Ordering::Acquire){return Err("REPORT.IMAGE_LIMIT_OR_CANCELLED".into());}
    let pixels=reader()?.decode().map_err(|e|e.to_string())?.into_luma8();
    let range=pixels.as_raw().iter().copied().fold((255u8,0u8),|(lo,hi),v|(lo.min(v),hi.max(v)));
    #[cfg(windows)] let recognition=recognize(&pixels).map_err(|e|e.to_string());
    #[cfg(not(windows))] let recognition:Result<String,String>=Err("No local OCR engine".into());
    if cancel.load(Ordering::Acquire){return Err("AGENT.CANCELLED".into());}
    let (text,error)=match recognition {Ok(v)=>(Some(v),None),Err(e)=>(None,Some(e))};
    Ok(json!({"source_id":path.file_name().unwrap_or_default().to_string_lossy(),"sha256":wiparse_core::update::hex_sha256(&bytes),
        "kind":"screenshot","width_px":w,"height_px":h,"luma_range":range,"ocr_text":text.as_ref().map(|v|v.chars().take(4096).collect::<String>()),
        "ocr_truncated":text.as_ref().is_some_and(|v|v.chars().count()>4096),"ocr_error":error,"calibrated_measurement":false,
        "interpretation":"OCR is a transcription of visible labels. Timing, voltage and pass/fail require calibrated source samples."}))
}

#[cfg(windows)]
fn recognize(pixels:&image::GrayImage)->windows::core::Result<String>{
    use windows::{Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap}, Media::Ocr::OcrEngine, Security::Cryptography::CryptographicBuffer};
    let buffer=CryptographicBuffer::CreateFromByteArray(pixels.as_raw())?;
    let bitmap=SoftwareBitmap::CreateCopyFromBuffer(&buffer,BitmapPixelFormat::Gray8,pixels.width() as i32,pixels.height() as i32)?;
    let result=OcrEngine::TryCreateFromUserProfileLanguages()?.RecognizeAsync(&bitmap)?.get()?;
    Ok(result.Text()?.to_string())
}
