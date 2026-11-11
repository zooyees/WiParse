use super::*;

fn parts(channels: u8, points: usize) -> Vec<(u8, Vec<u8>, String)> {
    (1..=channels).map(|ch| {
        let size = points.to_string();
        let mut raw = format!(":WFMPRE:BYT_NR 1;BIT_NR 8;ENCDG BIN;BN_FMT RI;BYT_OR MSB;NR_PT {points};WFID \"CH{ch}\";PT_FMT Y;XINCR 1E-6;PT_OFF 0;XZERO 0;XUNIT \"s\";YMULT 0.1;YZERO 0;YOFF 0;YUNIT \"V\";:CURVE #{}{size}", size.len()).into_bytes();
        raw.extend((0..points).map(|i| if (i / 32) % 2 == 0 { 30u8 } else { (-30i8) as u8 }));
        raw.push(b'\n');
        (ch, raw, format!("waveform_CH{ch}.isf"))
    }).collect()
}

fn source_event(channels: u8, points: usize) -> Event {
    let (tx, rx) = unbounded();
    process_source(
        SourceRequest {
            id: 1,
            job_id: Some(9),
            auto_dir: None,
            auto_filename: None,
            overwrite: false,
        },
        parts(channels, points),
        &tx,
    );
    rx.try_iter()
        .find(|event| matches!(event, Event::WaveformSource { .. }))
        .unwrap()
}

fn panel() -> InstrumentControlPanel {
    let mut panel = InstrumentControlPanel::new(&AppConfig::default());
    let device = open_demo_session(InstrumentKind::Oscilloscope).unwrap();
    panel.handle_event(&egui::Context::default(), connected_event(1, &device));
    panel.waveform_source_job = Some(9);
    panel
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wiparse-source-pipeline-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn waveform_source_pipeline_shares_all_channels_and_publishes_cached_metrics() {
    let mut panel = panel();
    let event = source_event(4, 4096);
    let (shared, expected_metrics) = match &event {
        Event::WaveformSource {
            traces,
            metrics,
            channels,
            ..
        } => {
            assert_eq!(channels, &[1, 2, 3, 4]);
            assert_eq!(metrics, &host_waveform_metrics(traces));
            (Arc::clone(traces), metrics.clone())
        }
        _ => unreachable!(),
    };
    let bus = crate::backend::EventBus::new();
    publish_instrument_event(&bus, &event, "fixture-epoch");
    let subscription = bus.subscribe(0).unwrap();
    let _ = subscription.recv().unwrap();
    let published: serde_json::Value = serde_json::from_str(&subscription.recv().unwrap()).unwrap();
    assert_eq!(published["data"]["metrics"], expected_metrics);
    panel.handle_event(&egui::Context::default(), event);
    let pending = panel.pending_waveform_save.as_ref().unwrap();
    assert!(Arc::ptr_eq(&shared, &pending.traces));
    assert!(Arc::ptr_eq(&shared[0].x, &panel.waveforms[&1].x));
    assert!(Arc::ptr_eq(&shared[0].y, &panel.waveforms[&1].y));
    assert_eq!(pending.traces.len(), 4);
    assert_eq!(pending.traces[0].channel, "CH1");
    assert_eq!(panel.wave_plots[&1].stats.count, 4096);
    assert_eq!(panel.waveform_source_job, Some(9));

    let dir = temp_dir();
    let pending = panel.pending_waveform_save.take().unwrap();
    let native = dir.join("native.isf");
    let csv = dir.join("all.csv");
    save_pending_waveform_source(&native, &pending).unwrap();
    save_pending_waveform_source(&csv, &pending).unwrap();
    assert_eq!(std::fs::read(&native).unwrap(), pending.bytes);
    for path in [&native, &csv] {
        let restored = wiparse_core::waveform_file::load_waveform_file_all(path).unwrap();
        assert_eq!(restored.len(), 4);
        for (old, new) in shared.iter().zip(restored.iter()) {
            assert_eq!(old.x.len(), new.x.len());
            assert_eq!(old.y, new.y);
        }
    }
    let (tx, rx) = unbounded();
    let csv_parts = shared
        .iter()
        .enumerate()
        .map(|(index, trace)| {
            (
                (index + 1) as u8,
                wiparse_core::waveform_file::waveform_to_wiparse_csv(trace),
                format!("CH{}.csv", index + 1),
            )
        })
        .collect();
    process_source(
        SourceRequest {
            id: 1,
            job_id: Some(9),
            auto_dir: None,
            auto_filename: None,
            overwrite: false,
        },
        csv_parts,
        &tx,
    );
    let csv_event = rx
        .try_iter()
        .find(|event| matches!(event, Event::WaveformSource { .. }))
        .unwrap();
    panel.handle_event(&egui::Context::default(), csv_event);
    let converted = panel.pending_waveform_save.take().unwrap();
    for extension in ["isf", "wfm"] {
        let selected = dir.join(format!("converted.{extension}"));
        let actual = save_pending_waveform_source(&selected, &converted).unwrap();
        assert_eq!(actual[0].extension().unwrap(), "isf");
        assert_eq!(
            wiparse_core::waveform_file::load_waveform_file_all(&actual[0])
                .unwrap()
                .len(),
            4
        );
    }
    let protected = dir.join("protected.isf");
    std::fs::write(&protected, b"keep-existing").unwrap();
    save_source(
        converted,
        SaveSelection::Auto {
            dir: dir.clone(),
            filename: Some("protected.wfm".into()),
            overwrite: false,
        },
        &tx,
    );
    assert!(matches!(
        rx.recv().unwrap(),
        Event::WaveformSaved {
            result: Ok(Some(_)),
            ..
        }
    ));
    assert_eq!(std::fs::read(&protected).unwrap(), b"keep-existing");
    assert!(dir.join("protected-2.isf").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn waveform_source_save_completion_and_errors_release_admission_gate() {
    let mut panel = panel();
    assert!(
        !panel
            .api_waveform_source(&serde_json::json!({ "device_id": 1 }))
            .ok
    );
    panel.handle_event(&egui::Context::default(), source_event(1, 256));
    let pending = panel.pending_waveform_save.take().unwrap();
    assert!(pending.channel_files[0].1.is_empty()); // no duplicate native buffer
    let dir = temp_dir();
    let path = dir.join("capture.isf");
    std::fs::write(&path, b"existing").unwrap();
    let (tx, rx) = unbounded();
    save_source(
        pending,
        SaveSelection::Auto {
            dir: dir.clone(),
            filename: Some("capture.isf".into()),
            overwrite: false,
        },
        &tx,
    );
    let completed = rx.recv().unwrap();
    assert!(path.exists());
    assert_eq!(std::fs::read(&path).unwrap(), b"existing");
    assert!(dir.join("capture-2.isf").exists());
    let original_save_dir = panel.save_dir.clone();
    panel.handle_event(&egui::Context::default(), completed);
    assert_eq!(panel.save_dir, original_save_dir); // automated saves do not change manual defaults
    assert_eq!(panel.waveform_source_job, None);
    let results = panel.take_job_results();
    assert_eq!(results.len(), 1);
    assert!(results[0].ok);
    assert!(results[0].detail["metrics"]["CH1"].is_object());

    panel.waveform_source_job = Some(10);
    panel.handle_event(
        &egui::Context::default(),
        Event::WaveformSaved {
            id: 1,
            job_id: Some(10),
            result: Ok(None),
            bytes: 0,
            metrics: serde_json::json!({}),
            update_save_dir: true,
        },
    );
    assert_eq!(panel.waveform_source_job, None);
    assert!(!panel.take_job_results()[0].ok);
    panel.waveform_source_job = Some(11);
    panel.handle_event(
        &egui::Context::default(),
        Event::Error {
            id: Some(1),
            job_id: Some(11),
            message: "USB detached".into(),
        },
    );
    assert_eq!(panel.waveform_source_job, None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn waveform_source_disconnected_result_does_not_resurrect_preview() {
    let mut panel = panel();
    panel.handle_event(&egui::Context::default(), Event::Disconnected(1));
    panel.handle_event(&egui::Context::default(), source_event(1, 256));
    assert!(panel.waveforms.is_empty());
    assert!(panel.pending_waveform_save.is_none());
    assert_eq!(panel.waveform_source_job, None);
    assert!(!panel.take_job_results()[0].ok);
}

#[test]
fn waveform_source_save_releases_other_channels_and_keeps_only_preview_samples() {
    let event = source_event(4, 512);
    let (first, second) = match &event {
        Event::WaveformSource { traces, .. } => {
            (Arc::downgrade(&traces[0].y), Arc::downgrade(&traces[1].y))
        }
        _ => unreachable!(),
    };
    let mut panel = panel();
    panel.handle_event(&egui::Context::default(), event);
    let pending = panel.pending_waveform_save.take().unwrap();
    let dir = temp_dir();
    let (tx, rx) = unbounded();
    save_source(
        pending,
        SaveSelection::Auto {
            dir: dir.clone(),
            filename: None,
            overwrite: false,
        },
        &tx,
    );
    panel.handle_event(&egui::Context::default(), rx.recv().unwrap());
    assert!(first.upgrade().is_some());
    assert!(second.upgrade().is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn waveform_source_malformed_data_can_be_saved_natively_and_reports_parse_error() {
    let (tx, rx) = unbounded();
    let original = b"bad source".to_vec();
    process_source(
        SourceRequest {
            id: 1,
            job_id: Some(9),
            auto_dir: None,
            auto_filename: None,
            overwrite: false,
        },
        vec![(1, original.clone(), "bad.isf".into())],
        &tx,
    );
    let event = rx
        .try_iter()
        .find(|event| matches!(event, Event::WaveformSource { .. }))
        .unwrap();
    if let Event::WaveformSource {
        parse_error,
        plot,
        ref bytes,
        ..
    } = &event
    {
        assert!(parse_error.is_some());
        assert!(plot.is_none());
        assert_eq!(bytes, &original);
    }
    let mut panel = panel();
    panel.handle_event(&egui::Context::default(), event);
    let pending = panel.pending_waveform_save.take().unwrap();
    let dir = temp_dir();
    let invalid_parent = dir.join("not-a-directory");
    std::fs::write(&invalid_parent, b"file").unwrap();
    save_source(
        pending,
        SaveSelection::Auto {
            dir: invalid_parent,
            filename: None,
            overwrite: false,
        },
        &tx,
    );
    let event = rx.recv().unwrap();
    assert!(matches!(
        &event,
        Event::WaveformSaved { result: Err(_), .. }
    ));
    panel.handle_event(&egui::Context::default(), event);
    assert_eq!(panel.waveform_source_job, None);
    assert!(!panel.take_job_results()[0].ok);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn waveform_source_processing_is_bounded_and_does_not_block_instrument_commands() {
    let (pipeline, _held_processing) = crossbeam_channel::bounded(2);
    for id in [1, 2] {
        pipeline
            .try_send(PipelineJob::Preview {
                id,
                trace: WaveformTrace {
                    channel: "CH1".into(),
                    x_unit: "s".into(),
                    y_unit: "V".into(),
                    x: vec![0.0, 1.0].into(),
                    y: vec![0.0, 1.0].into(),
                },
            })
            .unwrap_or_else(|_| panic!("queue unexpectedly full"));
    }
    assert!(pipeline
        .try_send(PipelineJob::Preview {
            id: 3,
            trace: WaveformTrace {
                channel: "CH1".into(),
                x_unit: "s".into(),
                y_unit: "V".into(),
                x: Vec::new().into(),
                y: Vec::new().into(),
            }
        })
        .is_err());
    let (jobs, jobs_rx) = unbounded();
    let (events, events_rx) = unbounded();
    let thread = std::thread::spawn(move || worker_loop(jobs_rx, events, pipeline));
    jobs.send(Job::ConnectDemo {
        id: 1,
        kind: InstrumentKind::Oscilloscope,
    })
    .unwrap();
    assert!(matches!(
        events_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Event::Connected { .. }
    ));
    jobs.send(Job::Command {
        id: 1,
        command: ControlCommand::RawQuery("*IDN?".into()),
        job_id: Some(15),
    })
    .unwrap();
    assert!(matches!(
        events_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Event::CommandDone {
            job_id: Some(15),
            ..
        }
    ));
    jobs.send(Job::Shutdown).unwrap();
    thread.join().unwrap();
}

#[test]
#[ignore = "performance fixture: run explicitly with --ignored --nocapture"]
fn waveform_source_million_point_ui_handoff_benchmark() {
    let mut panel = panel();
    let points = std::env::var("WIPARSE_WAVEFORM_BENCH_POINTS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1_000_000);
    let prepared = source_event(4, points);
    let shared = match &prepared {
        Event::WaveformSource { traces, .. } => Arc::clone(traces),
        _ => unreachable!(),
    };
    let baseline = Instant::now();
    std::hint::black_box(host_waveform_metrics(&shared));
    std::hint::black_box(host_waveform_metrics(&shared));
    std::hint::black_box(build_cached_wave_plot(&shared[0]));
    let old_ms = baseline.elapsed().as_secs_f64() * 1000.0;
    let bus = crate::backend::EventBus::new();
    let handoff = Instant::now();
    publish_instrument_event(&bus, &prepared, "fixture-epoch");
    panel.handle_event(&egui::Context::default(), prepared);
    let new_ms = handoff.elapsed().as_secs_f64() * 1000.0;
    assert!(Arc::ptr_eq(&shared[0].x, &panel.waveforms[&1].x));
    assert!(Arc::ptr_eq(&shared[0].y, &panel.waveforms[&1].y));
    assert_eq!(
        panel.pending_waveform_save.as_ref().unwrap().traces.len(),
        4
    );
    println!("WAVEFORM_BENCH {{\"channels\":4,\"points_per_channel\":{points},\"old_ui_work_ms\":{old_ms:.3},\"new_ui_handoff_ms\":{new_ms:.3},\"sample_buffers_shared\":true}}");
}
