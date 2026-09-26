use super::*;

/// Test-only observer of the production listener and snapshot queue. Payloads
/// stay in memory; only metadata and matches against explicitly supplied
/// synthetic fixtures leave this process. Never built into a shipping binary.
#[cfg(all(target_os = "windows", feature = "dev-harness"))]
pub fn run_capture_probe(
    manifest_path: &str,
    output_path: &str,
    seconds: u64,
) -> Result<(), String> {
    use std::io::Write;
    #[derive(serde::Deserialize)]
    struct Fixture {
        text: Option<String>,
        html_contains: Option<String>,
        html_exact: Option<String>,
        rtf_contains: Option<String>,
        rtf_exact: Option<String>,
        image_size: Option<[u32; 2]>,
        image_rgba: Option<Vec<u8>>,
        sensitive: Option<bool>,
    }
    // Capture logs carry sequence numbers and outcomes, never payloads. The
    // fixture scripts redirect stderr to a file, so a missed fixture leaves a
    // trace of why.
    struct StderrLog(Instant);
    impl log::Log for StderrLog {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.target().starts_with("cubby::clipboard")
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!(
                    "{:>7}ms {} {}",
                    self.0.elapsed().as_millis(),
                    record.level(),
                    record.args()
                );
            }
        }
        fn flush(&self) {}
    }
    if log::set_boxed_logger(Box::new(StderrLog(Instant::now()))).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
    let fixtures: Vec<Fixture> =
        serde_json::from_slice(&std::fs::read(manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut output = std::fs::File::create(output_path).map_err(|e| e.to_string())?;
    let (tx, rx) = snapshot_channel::channel();
    std::thread::spawn(move || run_native_listener(tx));
    let startup = Instant::now();
    while CAPTURE_STATE.load(Ordering::SeqCst) != CAPTURE_STATE_LISTENING {
        if startup.elapsed() > Duration::from_secs(5) {
            return Err("capture listener did not become ready".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    let result = runtime.block_on(async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
        writeln!(output, "{}", serde_json::json!({"event":"ready"})).map_err(|e| e.to_string())?;
        loop {
            let event = match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(event)) => event,
                _ => break,
            };
            if let ClipboardListenerEvent::Content(snapshot) = event {
                let matched: Vec<usize> = fixtures.iter().enumerate().filter_map(|(index, f)| {
                    let body_matches = match (&snapshot.content, &f.text, f.image_size) {
                        (CapturedContent::Text { content, .. }, Some(text), None) => content == text.as_bytes(),
                        (CapturedContent::Image { width, height, png_bytes, .. }, None, Some([w, h])) => {
                            *width == w && *height == h && f.image_rgba.as_ref().is_none_or(|expected| {
                                image::load_from_memory(png_bytes).is_ok_and(|image| image.to_rgba8().as_raw() == expected)
                            })
                        },
                        _ => false,
                    };
                    let format_matches = |name: &str, needle: &Option<String>| {
                        needle.as_ref().is_none_or(|needle| snapshot.formats.iter().any(|v|
                            v.name == name && String::from_utf8_lossy(&v.content).contains(needle)))
                    };
                    let format_exact = |name: &str, expected: &Option<String>| {
                        expected.as_ref().is_none_or(|expected| snapshot.formats.iter().any(|v|
                            v.name == name && v.content == expected.as_bytes()))
                    };
                    (body_matches && format_exact("html", &f.html_exact)
                        && format_exact("rtf", &f.rtf_exact)
                        && format_matches("html", &f.html_contains)
                        && format_matches("rtf", &f.rtf_contains)
                        && f.sensitive.is_none_or(|value| value == snapshot.sensitive.any()))
                        .then_some(index)
                }).collect();
                seen.extend(matched.iter().copied());
                writeln!(output, "{}", serde_json::json!({
                    "event":"capture", "sequence":snapshot.sequence,
                    "elapsed_ms":startup.elapsed().as_millis(),
                    "matches":matched, "format_count":snapshot.formats.len(),
                    "materialize_ms":snapshot.materialize_ms,
                    "bytes":snapshot.payload_bytes(), "sensitive":snapshot.sensitive.any(),
                })).map_err(|e| e.to_string())?;
            }
        }
        let missing: Vec<usize> = (0..fixtures.len()).filter(|i| !seen.contains(i)).collect();
        writeln!(output, "{}", serde_json::json!({"event":"summary","expected":fixtures.len(),"matched":seen.len(),"missing":missing})).map_err(|e| e.to_string())?;
        if missing.is_empty() { Ok(()) } else { Err(format!("{} synthetic fixtures were not captured", missing.len())) }
    });
    drop(rx);
    drop(LISTENER_SHUTDOWN.lock().take());
    result
}
