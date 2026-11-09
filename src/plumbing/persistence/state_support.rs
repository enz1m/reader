use super::*;

#[cfg(test)]
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub(crate) fn load_startup_library(resolver: &XdgResolver) -> RecoveryReload {
    let (store, outcome) = load_state_authority(resolver);
    RecoveryReload::Library { store, outcome }
}

#[cfg(test)]
type TestRawWriter =
    Box<dyn FnOnce(&mut dyn std::io::Write, &[u8]) -> std::io::Result<()> + 'static>;

#[cfg(test)]
thread_local! {
    static TEST_RAW_WRITER: std::cell::RefCell<Option<TestRawWriter>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
pub(crate) fn inject_raw_writer(
    write: impl FnOnce(&mut dyn std::io::Write, &[u8]) -> std::io::Result<()> + 'static,
) {
    TEST_RAW_WRITER.with(|slot| {
        assert!(slot.borrow().is_none(), "a raw writer is already injected");
        *slot.borrow_mut() = Some(Box::new(write));
    });
}

// Publish `bytes` as the complete contents of `path`, unframed: both loaders
// read raw JSON. An interrupted write leaves the previous file.
#[cfg(test)]
pub(crate) fn publish_raw(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(test)]
    if let Some(injected) = TEST_RAW_WRITER.with(|slot| slot.borrow_mut().take()) {
        return enzui_iced_core::atomic_state::save_atomic_raw_with(path, |staged| {
            injected(staged, bytes)
        });
    }

    enzui_iced_core::atomic_state::save_atomic_raw(path, bytes)
}
