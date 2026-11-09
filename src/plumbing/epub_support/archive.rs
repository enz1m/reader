use std::io::{Cursor, Read};

#[derive(Clone, Copy)]
struct Limits {
    encoded: u64,
    expanded: u64,
    entry: u64,
    entries: usize,
}

const LIMITS: Limits = Limits {
    encoded: 128 * 1024 * 1024,
    expanded: 256 * 1024 * 1024,
    entry: 32 * 1024 * 1024,
    entries: 8_192,
};

pub(super) fn open(path: &str) -> Result<epub::doc::EpubDoc<Cursor<Vec<u8>>>, String> {
    let file = std::fs::File::open(path).map_err(read_failure)?;
    if file.metadata().map_err(read_failure)?.len() > LIMITS.encoded {
        return Err("The EPUB exceeds the 128 MiB archive limit.".into());
    }
    let mut bytes = Vec::new();
    file.take(LIMITS.encoded + 1)
        .read_to_end(&mut bytes)
        .map_err(read_failure)?;
    admit(&bytes, LIMITS)?;
    // Parsing consumes the same immutable bytes whose expansion was checked.
    epub::doc::EpubDoc::from_reader(Cursor::new(bytes)).map_err(|error| {
        eprintln!("reader-app EPUB parse failure: {error:?}");
        "The EPUB structure could not be read.".to_string()
    })
}

fn read_failure(error: std::io::Error) -> String {
    eprintln!("reader-app EPUB read failure: {error:?}");
    match error.kind() {
        std::io::ErrorKind::NotFound => "The EPUB file could not be found.",
        std::io::ErrorKind::PermissionDenied => "You do not have permission to read this EPUB.",
        _ => "The EPUB file could not be read.",
    }
    .to_string()
}

fn admit(bytes: &[u8], limits: Limits) -> Result<(), String> {
    if bytes.len() as u64 > limits.encoded {
        return Err("The EPUB exceeds the archive byte limit.".into());
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "The EPUB archive is corrupt.".to_string())?;
    if archive.len() > limits.entries {
        return Err("The EPUB contains too many archive entries.".into());
    }
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| "An EPUB archive entry could not be read.".to_string())?;
        if !matches!(
            entry.compression(),
            zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
        ) {
            return Err("The EPUB uses an unsupported compression method.".into());
        }
        let remaining = limits.expanded.saturating_sub(expanded);
        let ceiling = limits.entry.min(remaining);
        if entry.size() > ceiling {
            return Err("The EPUB exceeds the expanded-content limit.".into());
        }
        let actual = std::io::copy(&mut entry.take(ceiling + 1), &mut std::io::sink())
            .map_err(|_| "An EPUB archive entry is corrupt.".to_string())?;
        if actual > ceiling {
            return Err("The EPUB exceeds the expanded-content limit.".into());
        }
        expanded += actual;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive(payloads: &[&[u8]]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (index, payload) in payloads.iter().enumerate() {
            writer
                .start_file(
                    index.to_string(),
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .expect("entry");
            writer.write_all(payload).expect("payload");
        }
        writer.finish().expect("archive").into_inner()
    }

    fn limits() -> Limits {
        Limits {
            encoded: 4096,
            expanded: 1024,
            entry: 768,
            entries: 4,
        }
    }

    #[test]
    fn epub_admission_bounds_encoded_input_entries_and_expansion() {
        {
            let data = vec![b'a'; 600];
            assert!(admit(&archive(&[&data]), limits()).is_ok());
            assert!(
                admit(&archive(&[&data, &data]), limits())
                    .unwrap_err()
                    .contains("expanded-content")
            );
            assert!(
                admit(&archive(&[&vec![b'a'; 800]]), limits())
                    .unwrap_err()
                    .contains("expanded-content")
            );
            assert!(
                admit(&archive(&[b"a", b"b", b"c", b"d", b"e"]), limits())
                    .unwrap_err()
                    .contains("too many")
            );
        }
        {
            let bytes = archive(&[b"valid resource"]);
            assert!(admit(&bytes, limits()).is_ok());
            assert!(
                admit(
                    &bytes,
                    Limits {
                        encoded: 1,
                        ..limits()
                    }
                )
                .unwrap_err()
                .contains("byte limit")
            );
            assert!(admit(b"not a zip", limits()).is_err());
        }
    }
}
