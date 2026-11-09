use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

fn fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-multipage.pdf")
}

fn tmpdir(label: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "reader-pdfops-{}-{}-{:?}",
        label,
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn scratch_pdf(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = tmpdir(label);
    let pdf = dir.join("doc.pdf");
    std::fs::copy(fixture(), &pdf).unwrap();
    (dir, pdf)
}

#[test]
fn pdf_mutations_preserve_file_authority_and_reject_aliased_destinations() {
    {
        {
            let (dir, pdf) = scratch_pdf("metadata-atomic");
            std::fs::set_permissions(&pdf, std::fs::Permissions::from_mode(0o600)).unwrap();
            write_metadata(&pdf, &PdfMetadata::default()).unwrap();
            assert_eq!(std::fs::metadata(&pdf).unwrap().mode() & 0o7777, 0o600);

            let alias = dir.join("metadata-alias.pdf");
            std::fs::hard_link(&pdf, &alias).unwrap();
            let before = std::fs::read(&pdf).unwrap();
            write_metadata(&pdf, &PdfMetadata::default()).unwrap_err();
            assert_eq!(std::fs::read(&pdf).unwrap(), before);
            assert_eq!(std::fs::read(&alias).unwrap(), before);
            let _ = std::fs::remove_dir_all(dir);
        }
        {
            let (dir, pdf) = scratch_pdf("clear-atomic");
            write_metadata(&pdf, &PdfMetadata::default()).unwrap();
            let alias = dir.join("clear-alias.pdf");
            std::fs::hard_link(&pdf, &alias).unwrap();
            let before = std::fs::read(&pdf).unwrap();

            clear_metadata(&pdf).unwrap_err();

            assert_eq!(std::fs::read(&pdf).unwrap(), before);
            assert_eq!(std::fs::read(&alias).unwrap(), before);
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    {
        let (dir, pdf) = scratch_pdf("extract-hard-link");
        let dest = dir.join("out.pdf");
        let alias = dir.join("out-alias.pdf");
        std::fs::write(&dest, b"previous bytes").unwrap();
        std::fs::hard_link(&dest, &alias).unwrap();

        extract_page(&pdf, 0, &dest).expect_err("hard-linked destination is refused");

        assert_eq!(std::fs::read(&dest).unwrap(), b"previous bytes");
        assert_eq!(std::fs::read(&alias).unwrap(), b"previous bytes");
        let _ = std::fs::remove_dir_all(dir);
    }
}
