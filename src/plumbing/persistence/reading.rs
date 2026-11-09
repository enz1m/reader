//! Raw JSON reads and test hooks.

use super::*;

#[derive(Clone, Copy)]
pub(crate) struct ReadIdentity {
    device: u64,
    inode: u64,
}

impl ReadIdentity {
    pub(crate) fn matches(
        self,
        retained: Option<enzui_iced_core::state_authority::StateIdentity>,
    ) -> bool {
        retained.is_some_and(|retained| {
            retained.device() == self.device && retained.inode() == self.inode
        })
    }
}

pub(crate) enum RawJsonRead {
    Missing,
    Body(Vec<u8>, ReadIdentity),
    Refused(StateRefusal, String, Option<ReadIdentity>),
}

pub(crate) fn read_raw_json(path: &Path, max_bytes: u64) -> RawJsonRead {
    let mut file = match enzui_iced_core::atomic_state::open_state_file_nofollow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return RawJsonRead::Missing,
        // The opener checks the descriptor itself and refuses a directory,
        // FIFO or device with `InvalidInput` before any read can block.
        Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
            return RawJsonRead::Refused(StateRefusal::NotRegularFile, error.to_string(), None);
        }
        Err(error) => {
            return RawJsonRead::Refused(StateRefusal::OpenRefused, error.to_string(), None);
        }
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return RawJsonRead::Refused(StateRefusal::StatFailed, error.to_string(), None);
        }
    };
    if !metadata.is_file() {
        return RawJsonRead::Refused(
            StateRefusal::NotRegularFile,
            "state path is not a regular file".into(),
            None,
        );
    }
    let identity = match read_identity(&metadata) {
        Ok(identity) => identity,
        Err(error) => {
            return RawJsonRead::Refused(StateRefusal::StatFailed, error.to_string(), None);
        }
    };
    if metadata.len() > max_bytes {
        if let Err(error) = enzui_iced_core::tempstage::verify_path_names_file(&file, path) {
            return RawJsonRead::Refused(StateRefusal::OpenRefused, error.to_string(), None);
        }
        return RawJsonRead::Refused(
            StateRefusal::TooLarge,
            "state file exceeds the byte limit".into(),
            Some(identity),
        );
    }
    #[cfg(test)]
    run_after_read_metadata_hook();
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    if let Err(error) = std::io::Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
    {
        let retained = enzui_iced_core::tempstage::verify_path_names_file(&file, path)
            .ok()
            .map(|()| identity);
        return RawJsonRead::Refused(StateRefusal::ReadFailed, error.to_string(), retained);
    }
    #[cfg(test)]
    run_after_json_read_hook();
    if let Err(error) = enzui_iced_core::tempstage::verify_path_names_file(&file, path) {
        return RawJsonRead::Refused(StateRefusal::OpenRefused, error.to_string(), None);
    }
    let observed_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let settled_len = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return RawJsonRead::Refused(
                StateRefusal::StatFailed,
                error.to_string(),
                Some(identity),
            );
        }
    };
    if observed_len > max_bytes {
        return RawJsonRead::Refused(
            StateRefusal::TooLarge,
            "state file grew past the byte limit".into(),
            Some(identity),
        );
    }
    if observed_len != metadata.len() || settled_len != metadata.len() {
        return RawJsonRead::Refused(
            StateRefusal::ReadFailed,
            "state file length changed during admission".into(),
            Some(identity),
        );
    }
    if bytes.is_empty() {
        return RawJsonRead::Refused(
            StateRefusal::Empty,
            "state file is empty".into(),
            Some(identity),
        );
    }
    RawJsonRead::Body(bytes, identity)
}

#[cfg(unix)]
fn read_identity(metadata: &fs::Metadata) -> std::io::Result<ReadIdentity> {
    use std::os::unix::fs::MetadataExt;

    Ok(ReadIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn read_identity(_metadata: &fs::Metadata) -> std::io::Result<ReadIdentity> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "state identity requires Unix",
    ))
}

#[cfg(test)]
thread_local! {
    static AFTER_READ_METADATA: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
    static AFTER_JSON_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
    static AFTER_RETAINED_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
pub(crate) fn arm_after_read_metadata_hook(action: impl FnOnce() + 'static) {
    AFTER_READ_METADATA.with(|slot| *slot.borrow_mut() = Some(Box::new(action)));
}

#[cfg(test)]
fn run_after_read_metadata_hook() {
    AFTER_READ_METADATA.with(|slot| {
        if let Some(action) = slot.borrow_mut().take() {
            action();
        }
    });
}

#[cfg(test)]
pub(crate) fn arm_after_json_read_hook(action: impl FnOnce() + 'static) {
    AFTER_JSON_READ.with(|slot| *slot.borrow_mut() = Some(Box::new(action)));
}

#[cfg(test)]
fn run_after_json_read_hook() {
    AFTER_JSON_READ.with(|slot| {
        if let Some(action) = slot.borrow_mut().take() {
            action();
        }
    });
}

#[cfg(test)]
pub(crate) fn arm_after_retained_read_hook(action: impl FnOnce() + 'static) {
    AFTER_RETAINED_READ.with(|slot| *slot.borrow_mut() = Some(Box::new(action)));
}

#[cfg(test)]
pub(crate) fn run_after_retained_read_hook() {
    AFTER_RETAINED_READ.with(|slot| {
        if let Some(action) = slot.borrow_mut().take() {
            action();
        }
    });
}
