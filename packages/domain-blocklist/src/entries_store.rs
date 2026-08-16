//! On-disk scratch representation of a merged entry corpus, so the sweep never needs the whole
//! `Vec<MergedEntry>` resident for its 10+ hour duration. `sweep::run_sweep_streaming` only ever
//! reads this corpus sequentially — once to gather due-domain batches, once more for the final
//! pruning scan — so a plain length-prefixed record file, reopened per pass, is sufficient; there
//! is no random access to serve and so no need for an index or an mmap.
//!
//! `MergedEntry` itself has no serde derives (same reasoning `cache_store.rs` gives for
//! `CacheEntry`: adding them means editing an already-shipped module 2 file for a module 7
//! concern), so this defines its own DTO. `SourceId`/`Category`/`RuleScope` already derive
//! `Serialize`/`Deserialize` and are reused directly.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use domain_blocklist::{Category, MergedEntry, SourceId};
use domain_normalize::RuleScope;

#[derive(serde::Serialize, serde::Deserialize)]
struct MergedEntryDto {
    domain: String,
    scope: RuleScope,
    sources: Vec<SourceId>,
    categories: Vec<Category>,
}

impl From<&MergedEntry> for MergedEntryDto {
    fn from(e: &MergedEntry) -> Self {
        Self {
            domain: e.domain.clone(),
            scope: e.scope,
            sources: e.sources.clone(),
            categories: e.categories.clone(),
        }
    }
}

impl From<MergedEntryDto> for MergedEntry {
    fn from(d: MergedEntryDto) -> Self {
        Self {
            domain: d.domain,
            scope: d.scope,
            sources: d.sources,
            categories: d.categories,
        }
    }
}

/// Writes `entries` sequentially, in the order given (merge already produces sorted order — this
/// preserves it, never re-sorts). One record serialized at a time, so writing itself never holds
/// more than the caller's own already-resident `entries` slice plus one record's bytes.
pub fn write(path: &Path, entries: &[MergedEntry]) -> anyhow::Result<()> {
    let mut w = BufWriter::new(
        File::create(path)
            .map_err(|e| anyhow::anyhow!("failed to create {}: {e}", path.display()))?,
    );
    for entry in entries {
        let bytes = bincode::serialize(&MergedEntryDto::from(entry))
            .map_err(|e| anyhow::anyhow!("failed to serialize entry {:?}: {e}", entry.domain))?;
        w.write_all(&(bytes.len() as u64).to_le_bytes())?;
        w.write_all(&bytes)?;
    }
    w.flush()?;
    Ok(())
}

/// Sequential, forward-only reader over a file [`write`] produced. Every [`open`](Self::open)
/// starts a fresh pass from the top — reopening a plain file is simpler than a seekable cursor,
/// and the sweep only ever needs full passes, never a seek to an arbitrary offset.
pub struct EntryReader {
    reader: BufReader<File>,
}

impl EntryReader {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            reader: BufReader::new(
                File::open(path)
                    .map_err(|e| anyhow::anyhow!("failed to open {}: {e}", path.display()))?,
            ),
        })
    }

    /// Returns the next entry, or `None` at a clean end of file. Any other I/O or deserialization
    /// failure is a hard error — a truncated scratch file must never be read as "fewer entries
    /// than were written."
    pub fn next(&mut self) -> anyhow::Result<Option<MergedEntry>> {
        let mut len_bytes = [0u8; 8];
        match self.reader.read_exact(&mut len_bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(anyhow::anyhow!("failed to read entry length: {e}")),
        }
        let len = u64::from_le_bytes(len_bytes) as usize;
        let mut buf = vec![0u8; len];
        self.reader
            .read_exact(&mut buf)
            .map_err(|e| anyhow::anyhow!("truncated entry record ({len} bytes expected): {e}"))?;
        let dto: MergedEntryDto = bincode::deserialize(&buf)
            .map_err(|e| anyhow::anyhow!("failed to deserialize entry record: {e}"))?;
        Ok(Some(dto.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain_blocklist::MergedEntry;

    fn entry(
        domain: &str,
        scope: RuleScope,
        sources: &[SourceId],
        categories: &[Category],
    ) -> MergedEntry {
        MergedEntry {
            domain: domain.to_string(),
            scope,
            sources: sources.to_vec(),
            categories: categories.to_vec(),
        }
    }

    #[test]
    fn round_trips_every_field_across_multiple_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("entries.bin");

        let entries = vec![
            entry(
                "adult-example.com",
                RuleScope::Apex,
                &[SourceId::StevenBlack, SourceId::Hagezi],
                &[Category::Adult],
            ),
            entry(
                "gambling.example.net",
                RuleScope::ExactHost,
                &[SourceId::Ut1],
                &[Category::Gambling, Category::Dating],
            ),
            entry("no-sources.example.org", RuleScope::ExactHost, &[], &[]),
        ];

        write(&path, &entries).unwrap();

        let mut reader = EntryReader::open(&path).unwrap();
        let mut read_back = Vec::new();
        while let Some(e) = reader.next().unwrap() {
            read_back.push(e);
        }
        assert_eq!(read_back, entries);
    }

    #[test]
    fn an_empty_corpus_round_trips_to_zero_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("entries.bin");
        write(&path, &[]).unwrap();
        let mut reader = EntryReader::open(&path).unwrap();
        assert!(reader.next().unwrap().is_none());
    }

    #[test]
    fn opening_a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.bin");
        assert!(EntryReader::open(&path).is_err());
    }

    #[test]
    fn a_truncated_record_is_a_hard_error_not_a_silent_short_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("entries.bin");
        write(
            &path,
            &[entry("example.com", RuleScope::ExactHost, &[], &[])],
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 2]).unwrap();

        let mut reader = EntryReader::open(&path).unwrap();
        assert!(reader.next().is_err());
    }

    /// Two independent [`EntryReader::open`] passes over the same file each see every record —
    /// the exact property [`sweep::run_sweep_streaming`] depends on for its two-pass design.
    #[test]
    fn two_independent_passes_each_see_the_full_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("entries.bin");
        let entries = vec![
            entry("a.example", RuleScope::ExactHost, &[], &[]),
            entry("b.example", RuleScope::ExactHost, &[], &[]),
        ];
        write(&path, &entries).unwrap();

        for _ in 0..2 {
            let mut reader = EntryReader::open(&path).unwrap();
            let mut count = 0;
            while reader.next().unwrap().is_some() {
                count += 1;
            }
            assert_eq!(count, 2);
        }
    }
}
