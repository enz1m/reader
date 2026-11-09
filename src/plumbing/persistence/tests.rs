use super::*;
fn tmpdir() -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "reader-app-persist-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    base
}

mod compat_tests;

mod sidecar_tests;

mod theme_tests;
