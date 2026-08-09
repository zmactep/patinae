//! Compression/decompression support
//!
//! Provides transparent handling of gzip-compressed files.

use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

use crate::error::IoResult;

/// Check if a file is gzip compressed by looking at magic bytes
pub fn is_gzip<R: Read>(reader: &mut R) -> std::io::Result<([u8; 2], bool)> {
    let mut magic = [0u8; 2];
    reader.read_exact(&mut magic)?;
    Ok((magic, magic[0] == 0x1f && magic[1] == 0x8b))
}

/// Check if a path indicates a gzip file (by extension)
pub fn is_gzip_path(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("gz"))
        .unwrap_or(false)
}

/// Reader that transparently handles gzip compression
pub enum MaybeGzReader<R: Read> {
    /// Plain uncompressed reader
    Plain(R),
    /// Gzip-compressed reader
    Gzip(GzDecoder<R>),
}

impl<R: Read> Read for MaybeGzReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            MaybeGzReader::Plain(r) => r.read(buf),
            MaybeGzReader::Gzip(r) => r.read(buf),
        }
    }
}

/// Writer that can optionally gzip-compress output
pub enum MaybeGzWriter<W: Write> {
    /// Plain uncompressed writer
    Plain(W),
    /// Gzip-compressed writer
    Gzip(GzEncoder<W>),
}

impl<W: Write> Write for MaybeGzWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            MaybeGzWriter::Plain(w) => w.write(buf),
            MaybeGzWriter::Gzip(w) => w.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            MaybeGzWriter::Plain(w) => w.flush(),
            MaybeGzWriter::Gzip(w) => w.flush(),
        }
    }
}

impl<W: Write> MaybeGzWriter<W> {
    /// Finish compression and return the underlying writer
    pub fn finish(self) -> std::io::Result<W> {
        match self {
            MaybeGzWriter::Plain(w) => Ok(w),
            MaybeGzWriter::Gzip(w) => w.finish(),
        }
    }
}

/// Open a file for reading, automatically detecting and handling gzip compression
pub fn open_file(path: &Path) -> IoResult<Box<dyn Read>> {
    let file = File::open(path)?;

    if is_gzip_path(path) {
        Ok(Box::new(GzDecoder::new(file)))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

/// Create a gzip decoder for a reader
pub fn gzip_reader<R: Read>(reader: R) -> GzDecoder<R> {
    GzDecoder::new(reader)
}

/// Create a gzip encoder for a writer
pub fn gzip_writer<W: Write>(writer: W) -> GzEncoder<W> {
    GzEncoder::new(writer, Compression::default())
}

/// Create a gzip encoder with a specific compression level
pub fn gzip_writer_level<W: Write>(writer: W, level: u32) -> GzEncoder<W> {
    GzEncoder::new(writer, Compression::new(level))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[derive(Debug)]
    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected write failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_is_gzip_path() {
        assert!(is_gzip_path(Path::new("file.pdb.gz")));
        assert!(is_gzip_path(Path::new("file.GZ")));
        assert!(!is_gzip_path(Path::new("file.pdb")));
    }

    #[test]
    fn test_gzip_roundtrip() {
        let original = b"Hello, World! This is a test of gzip compression.";

        // Compress
        let mut compressed = Vec::new();
        {
            let mut encoder = gzip_writer(&mut compressed);
            encoder.write_all(original).unwrap();
            encoder.finish().unwrap();
        }

        // Decompress
        let mut decompressed = Vec::new();
        {
            let mut decoder = gzip_reader(Cursor::new(&compressed));
            decoder.read_to_end(&mut decompressed).unwrap();
        }

        assert_eq!(original.as_slice(), decompressed.as_slice());
    }

    #[test]
    fn gzip_finish_propagates_output_failure() {
        let writer = MaybeGzWriter::Gzip(gzip_writer(FailingWriter));

        let error = writer
            .finish()
            .expect_err("gzip finalization failure should be returned");

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
    }
}
