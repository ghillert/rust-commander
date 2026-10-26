# Local patch to sevenz-rust 0.6.1

Source: https://crates.io/crates/sevenz-rust/0.6.1
Upstream: https://github.com/dyz1990/sevenz-rust
License: Apache-2.0 (see LICENSE).

The published crate's source, manifest, README and license are retained; examples
and tests are omitted, with their manifest entries removed.

`src/writer/unpack_info.rs`: omit the SubStreamsInfo CRC property when all
substreams inherit their folder CRC. The original writer emits an empty digest
list, rejected by libarchive's `read_Digests` (including macOS bsdtar and the
Windows/Linux library). CRC zero is a defined checksum, not a missing checksum.
Solid folders with multiple substreams retain their individual checksums.

`src/writer/pack_info.rs`: all streams have calculated CRCs. Emit their values,
including zero, rather than emitting an incomplete conditional digest block.

`src/writer.rs`: omit MainStreamsInfo for archives containing only empty files or
directories. Such archives have no packed streams. Correct the inverted anti-item
flags: ordinary directories must not be marked as deletion records. Official
7-Zip otherwise omits those directories during extraction, unlike libarchive.

`src/archive.rs`: populate directory/archive/read-only attributes in from_path.
Without the directory attribute, libarchive extracts directories without search
permission, making their children inaccessible to ordinary users.

`src/reader.rs` and `src/folder.rs`: remove an unused private trait and spell out
an elided iterator lifetime to keep current Rust builds free of warnings.

Regression coverage: `tests/packing.rs` independently extracts generated archives
with libarchive via compress-tools, rather than the sevenz-rust reader. Keep this
patch until an upstream release resolves the interoperability issue.
