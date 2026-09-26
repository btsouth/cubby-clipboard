#[cfg(target_os = "windows")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!(
            "usage: capture_probe <synthetic-fixture-manifest.json> <metadata.jsonl> <seconds>"
        );
        std::process::exit(2);
    }
    let result = args[3]
        .parse()
        .map_err(|e| format!("invalid timeout: {e}"))
        .and_then(|seconds| cubby::run_capture_probe(&args[1], &args[2], seconds));
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
#[cfg(not(target_os = "windows"))]
fn main() {
    panic!("Windows only");
}
