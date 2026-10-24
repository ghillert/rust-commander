//! Stream a local file into an archive and print elapsed time, size and progress.
//! Run with: pack_benchmark SOURCE DESTINATION [default|0..9] [timeout-seconds] [history].
use rust_commander::{
    archive, history,
    operations::{Operation, Task},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|value| value == "--verify-7z") {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        anyhow::ensure!(args.len() == 3, "Use --verify-7z SOURCE ARCHIVE");
        let source = PathBuf::from(&args[1]);
        let hash = |input: &mut dyn Read| -> std::io::Result<Vec<u8>> {
            let mut digest = Sha256::new();
            let mut buffer = [0; 65536];
            loop {
                let count = input.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
            Ok(digest.finalize().to_vec())
        };
        let expected = hash(&mut std::fs::File::open(&source)?)?;
        let mut reader = sevenz_rust::SevenZReader::open(
            PathBuf::from(&args[2]),
            sevenz_rust::Password::empty(),
        )?;
        let mut actual = None;
        reader.for_each_entries(|entry, input| {
            if entry.name == source.file_name().unwrap_or_default().to_string_lossy() {
                actual = Some(hash(input).map_err(sevenz_rust::Error::io)?);
            }
            Ok(true)
        })?;
        anyhow::ensure!(
            actual.as_ref() == Some(&expected),
            "Archive contents do not match the source file"
        );
        println!("SHA-256 verified against source");
        return Ok(());
    }
    anyhow::ensure!(
        args.len() >= 2,
        "Provide a source file and a new archive path."
    );
    let source = PathBuf::from(&args[0]);
    let destination = PathBuf::from(&args[1]);
    let level = args
        .get(2)
        .and_then(|s| s.to_str())
        .filter(|s| *s != "default")
        .map(str::parse::<u8>)
        .transpose()?;
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(seconds) = args
        .get(3)
        .and_then(|s| s.to_str())
        .map(str::parse::<u64>)
        .transpose()?
    {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(seconds));
            cancel.store(true, Ordering::Relaxed);
        });
    }
    let mut task = Task::new(cancel, |_| {});
    let start = Instant::now();
    let with_history = args.get(4).is_some_and(|value| value == "history");
    let result = if with_history {
        history::execute_in(
            Operation::Pack {
                sources: vec![source.clone()],
                destination: destination.clone(),
                comment: String::new(),
                background: true,
                compression_level: level,
            },
            &mut task,
            &destination
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("history"),
        )
        .and_then(|record| {
            anyhow::ensure!(record.success, "{}", record.result);
            Ok(())
        })
    } else {
        archive::pack_with_compression(
            std::slice::from_ref(&source),
            &destination,
            "",
            level,
            &mut task,
        )
    };
    println!(
        "{}",
        serde_json::json!({"seconds":start.elapsed().as_secs_f64(), "source_bytes":std::fs::metadata(&source)?.len(), "processed_bytes":task.progress.bytes, "archive_bytes":std::fs::metadata(&destination).ok().map(|m|m.len()), "compression_level":level, "with_history":with_history, "error":result.err().map(|e|format!("{e:#}"))})
    );
    Ok(())
}
