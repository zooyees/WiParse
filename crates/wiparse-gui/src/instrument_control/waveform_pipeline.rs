//! Bounded CPU/file stage. VISA ownership remains on the instrument worker.
use super::*;

pub(super) struct SourceRequest {
    pub id: u64,
    pub job_id: Option<u64>,
    pub auto_dir: Option<PathBuf>,
    pub auto_filename: Option<String>,
    pub overwrite: bool,
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub secure_destination: Option<secure_output::Destination>,
}

pub(super) enum PipelineJob {
    Source {
        request: SourceRequest,
        parts: Vec<(u8, Vec<u8>, String)>,
    },
    Preview {
        id: u64,
        trace: WaveformTrace,
    },
    Save {
        pending: PendingWaveformSave,
        selection: SaveSelection,
    },
}

pub(super) enum SaveSelection {
    Dialog {
        dir: PathBuf,
    },
    Auto {
        dir: PathBuf,
        filename: Option<String>,
        overwrite: bool,
    },
    Secure { destination: secure_output::Destination },
}

pub(super) fn worker(jobs: Receiver<PipelineJob>, events: Sender<Event>) {
    while let Ok(job) = jobs.recv() {
        match job {
            PipelineJob::Source { request, parts } => process_source(request, parts, &events),
            PipelineJob::Preview { id, trace } => {
                let plot = build_cached_wave_plot(&trace);
                let _ = events.send(Event::Waveform { id, trace, plot });
            }
            PipelineJob::Save { pending, selection } => save_source(pending, selection, &events),
        }
    }
}

fn save_source(pending: PendingWaveformSave, selection: SaveSelection, events: &Sender<Event>) {
    let update_save_dir = matches!(&selection, SaveSelection::Dialog { .. });
    let secure_archive = matches!(&selection, SaveSelection::Secure { .. });
    let result = (|| -> Result<Option<Vec<PathBuf>>, String> {
        if pending.cancel.as_ref().is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err("AI.CANCELLED_BEFORE_SAVE".into());
        }
        let path = match selection {
            SaveSelection::Secure { destination } => {
                if pending.conversion_incomplete {
                    return Err("AI.EVIDENCE_INCOMPLETE_CHANNELS: multi-channel conversion omitted unreadable channels".into());
                }
                if pending.bytes.is_empty() { return Err("AI.EVIDENCE_NO_WAVEFORM_BYTES".into()); }
                if pending.cancel.as_ref().is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
                    return Err("AI.CANCELLED_BEFORE_SAVE".into());
                }
                return destination.write_new(&pending.bytes, &pending.effective_ext, pending.cancel.as_deref()).map(|path| Some(vec![path]));
            }
            SaveSelection::Dialog { dir } => {
                let name = format!(
                    "{}_{}.{}",
                    pending.file_stem,
                    Local::now().format("%Y%m%d_%H%M%S"),
                    pending.effective_ext
                );
                // rfd's Windows backend initializes COM on the calling thread.
                // Keep the dialog and file conversion off the egui thread.
                let Some(path) = scope_waveform_save_dialog(
                    &pending.vendor,
                    &dir,
                    &name,
                    &pending.effective_ext,
                )
                .save_file() else {
                    return Ok(None);
                };
                if pending.traces.len() > 1
                    && path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("wfm"))
                    && path.with_extension("isf").exists()
                {
                    return Err("Multi-channel WFM uses ISF; choose the .isf target explicitly to confirm replacing the existing file".into());
                }
                path
            }
            SaveSelection::Auto {
                dir,
                filename,
                overwrite,
            } => {
                std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
                let mut name = filename
                    .unwrap_or_else(|| format!("{}.{}", pending.file_stem, pending.effective_ext));
                if pending.traces.len() > 1
                    && std::path::Path::new(&name)
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("wfm"))
                {
                    name = std::path::Path::new(&name)
                        .with_extension("isf")
                        .to_string_lossy()
                        .into_owned();
                }
                wiparse_core::paths::unique_file_path(&dir, &name, overwrite)
            }
        };
        // The native dialog may have remained open after task cancellation.
        // Do not start a file write after it returns in that case.
        if pending.cancel.as_ref().is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err("AI.CANCELLED_BEFORE_SAVE".into());
        }
        save_pending_waveform_source(&path, &pending).map(Some)
    })();
    let project_relative = if secure_archive {
        result.as_ref().ok().and_then(|saved| saved.as_ref()).and_then(|saved| saved.first())
            .and_then(|path| path.file_name()).map(|name| PathBuf::from("ai-evidence").join(name))
    } else { None };
    let _ = events.send(Event::WaveformSaved {
        id: pending.id,
        job_id: pending.job_id,
        result,
        bytes: pending.bytes.len(),
        metrics: pending.metrics,
        update_save_dir,
        project_relative,
        source_parse_error: pending.source_parse_error,
        source_representation: pending.source_representation,
        source_channels: pending.source_channels,
    });
}

fn process_source(
    request: SourceRequest,
    parts: Vec<(u8, Vec<u8>, String)>,
    events: &Sender<Event>,
) {
    let started = Instant::now();
    let SourceRequest {
        id,
        job_id,
        auto_dir,
        auto_filename,
        overwrite,
        cancel,
        secure_destination,
    } = request;
    if cancel.as_ref().is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
        send_error_job(events, Some(id), job_id, "AI.CANCELLED_BEFORE_PROCESSING");
        return;
    }
    let mut channel_files = Vec::new();
    let mut errors = Vec::new();
    let mut total_bytes = 0usize;
    for (ch, bytes, suggested_name) in parts {
        if cancel.as_ref().is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            send_error_job(events, Some(id), job_id, "AI.CANCELLED_DURING_PROCESSING");
            return;
        }
        total_bytes += bytes.len();
        let _ = events.send(Event::Progress {
            id: Some(id),
            message: format!(
                "正在处理 CH{ch}（{} KB）/ Processing CH{ch} ({} KB)",
                bytes.len() / 1024,
                bytes.len() / 1024
            ),
        });
        let stem = std::path::PathBuf::from(&suggested_name);
        let ext = stem
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("isf")
            .to_ascii_lowercase();
        channel_files.push((ch, bytes, ext));
    }
    if channel_files.is_empty() {
        send_error_job(
            &events,
            Some(id),
            job_id,
            "no displayed channel waveform data",
        );
    } else {
        let ch_tag = channel_files
            .iter()
            .map(|(ch, _, _)| format!("CH{ch}"))
            .collect::<Vec<_>>()
            .join("_");
        let all_isf = channel_files.len() > 1 && channel_files.iter().all(|(_, _, e)| e == "isf");
        let (ch0, raw0, ext0) = &channel_files[0];
        let (bytes, suggested_name) = if channel_files.len() == 1 {
            (Vec::new(), format!("waveform_CH{ch0}.{ext0}"))
        } else if all_isf {
            let parts: Vec<(u8, &[u8])> = channel_files
                .iter()
                .map(|(ch, b, _)| (*ch, b.as_slice()))
                .collect();
            (
                join_tek_isf_channels(&parts),
                format!("waveform_{ch_tag}.isf"),
            )
        } else {
            (Vec::new(), format!("waveform_{ch_tag}.csv"))
        };

        let traces = if channel_files.len() == 1 {
            match load_waveform_bytes_all(raw0, ext0, &format!("CH{ch0}")).or_else(|first| {
                match sniff_waveform_ext(raw0) {
                    Some(sniffed) if sniffed != *ext0 => {
                        load_waveform_bytes_all(raw0, sniffed, &format!("CH{ch0}"))
                    }
                    _ => Err(first),
                }
            }) {
                Ok(t) => t,
                Err(e) => {
                    errors.push(format!("CH{ch0}: {e}"));
                    Vec::new()
                }
            }
        } else if all_isf {
            match load_waveform_bytes_all(&bytes, "isf", "CH1") {
                Ok(t) => t,
                Err(e) => {
                    errors.push(e.to_string());
                    Vec::new()
                }
            }
        } else {
            let mut t = Vec::new();
            for (ch, b, e) in &channel_files {
                match load_waveform_bytes(b, e, &format!("CH{ch}")).or_else(|first| {
                    match sniff_waveform_ext(b) {
                        Some(sniffed) if sniffed != *e => {
                            load_waveform_bytes(b, sniffed, &format!("CH{ch}"))
                        }
                        _ => Err(first),
                    }
                }) {
                    Ok(tr) => t.push(tr),
                    Err(e) => errors.push(format!("CH{ch}: {e}")),
                }
            }
            t
        };

        let (bytes, suggested_name) = if channel_files.len() == 1 {
            (std::mem::take(&mut channel_files[0].1), suggested_name)
        } else if !all_isf && !traces.is_empty() {
            (
                waveforms_to_spreadsheet_csv(&traces),
                format!("waveform_{ch_tag}.csv"),
            )
        } else {
            (bytes, suggested_name)
        };
        let parse_error = if traces.is_empty() {
            Some(errors.join("; "))
        } else if !errors.is_empty() {
            Some(format!("partial: {}", errors.join("; ")))
        } else {
            None
        };
        // Save owns the complete channel set; display shares CH1's Arc sample axes.
        // Once joined, keep no duplicate native channel blobs.
        let channels = channel_files.iter().map(|(ch, _, _)| *ch).collect();
        let plot = traces.first().map(build_cached_wave_plot);
        let metrics = host_waveform_metrics(&traces);
        let keep_parts = channel_files.len() <= 1;
        let _ = events.send(Event::WaveformSource {
            id,
            job_id,
            auto_dir,
            auto_filename,
            overwrite,
            cancel,
            secure_destination,
            bytes,
            suggested_name,
            plot,
            metrics,
            channels,
            traces: Arc::new(traces),
            channel_files: if keep_parts {
                channel_files
            } else {
                Vec::new()
            },
            parse_error,
        });
        let _ = total_bytes;
    }
    tracing::info!(
        device_id = id,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "waveform source processing complete (background)"
    );
}

#[cfg(test)]
mod tests;
