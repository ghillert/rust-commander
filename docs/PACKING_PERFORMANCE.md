# Packing performance and compression levels

[← File operations](FILE_OPERATIONS.md#archives)

The Pack files dialog and encrypted archive creation support explicit compression
levels. Default preserves the encoder's normal setting. ZIP/GZip and 7z/XZ offer
0–9; BZip2 offers 1–9. Compressed TAR variants use their corresponding compressor.
Plain TAR has no compression level.

- ZIP level 0 stores files without compression.
- GZip level 0 writes an uncompressed deflate stream.
- 7z and XZ level 0 select their fastest compression preset, not storage.
- Higher levels usually use more CPU time and memory to produce smaller archives.

## Development build performance

The macOS development app previously compiled its compression and hashing libraries
without optimization. This made large ZIP files much slower than Finder and was
especially expensive for the Rust LZMA2 encoder used by 7z.

The development and release profiles now optimize `zlib-rs`, `lzma-rust`,
`sevenz-rust`, `crc` and `sha2` for speed. Application/UI code retains the usual
settings for its profile. ZIP explicitly enables the flate2 backend, with
`zlib-rs` selected directly rather than through an unrelated dependency. The
unused Zopfli encoder is no longer enabled.

Rebuild and restart the development bundle with `./scripts/run.sh` to use the
new code. An already-running app keeps its existing executable.

## Measured example: 507 MB CSV

Measurements on the available Apple Silicon Mac, 2026-10-23, using the user's
`wcvp_taxon.csv` (507,635,435 bytes). These are individual local runs with warm
filesystem caches, not release-build or cross-platform performance guarantees.
`ditto` is a native macOS ZIP comparison, not a measurement of Finder's UI.

| Workflow | Time | Archive size |
| --- | ---: | ---: |
| macOS `ditto` ZIP | 5.4 s | 80.7 MB |
| Optimized dev ZIP, Default, including undo/history | 3.6 s | 80.3 MB |
| Optimized dev ZIP, level 1, including undo/history | 1.9 s | 116.1 MB |
| Optimized dev 7z, level 0 / Fastest, encoding only | 12.3 s | 67.0 MB |
| Optimized dev 7z, Default, encoding only | 159.4 s | 51.2 MB |

Before optimization, a bounded ZIP run processed only 405,667,840 bytes in 45
seconds, and a 7z run processed 12,353,536 bytes in 20 seconds. Those two runs
were cancelled at their time limits and did not produce completed archives.

Default 7z remains much slower than ZIP on this data because it spends more work
finding compression matches and produces a substantially smaller archive. Choose
7z level 0 when packing speed matters more than the smallest output.

ZIP output was independently extracted and compared against the original with
SHA-256. The full Default and Fastest 7z outputs also matched the source through the
built-in reader. An external-interoperability gap remains: both tested `bsdtar` versions
reject the generated 7z archives, including small fixtures. Built-in round trips
are not evidence of full compatibility with external 7z readers.

## Reproducing a benchmark

Use a new destination each time. All output and recovery files in this example
are stored under a disposable temporary directory:

```sh
./scripts/cargo-local.sh build --locked --example pack_benchmark
benchmark_dir=$(mktemp -d)
target/debug/examples/pack_benchmark /path/to/input.csv "$benchmark_dir/default.zip" default 60 history
target/debug/examples/pack_benchmark /path/to/input.csv "$benchmark_dir/fast.7z" 0 60
```

The optional timeout cancels through the same worker cancellation flag. JSON
reports include elapsed seconds, processed bytes, output size and errors. A
cancelled result must not be treated as a completed-file timing. `history` adds
normal undo bookkeeping and verification. Without it, timings cover encoding and
atomic publication only. UI rendering is not part of these timings.

To compare a single-file 7z archive against its source without extracting a copy:

```sh
target/debug/examples/pack_benchmark --verify-7z /path/to/input.csv "$benchmark_dir/fast.7z"
```
