//! A per-test scratch directory.

use std::fs;
use std::path::Path;

use omnia::Mount;
use tempfile::TempDir;

/// A per-test directory, removed on drop — including when the test panics
/// partway through.
///
/// ```
/// let scratch = omnia_test::host::scratch();
/// scratch.write("in/config.toml", "answer = 42");
/// let mount = scratch.mount(true);
/// assert_eq!((mount.name.as_str(), mount.writable), (".", true));
/// assert_eq!(scratch.read("in/config.toml"), Some(b"answer = 42".to_vec()));
/// ```
#[derive(Debug)]
pub struct Scratch(TempDir);

impl Scratch {
    /// The directory path.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.0.path()
    }

    /// A [`Mount`] preopening this directory into the guest sandbox as `.`.
    #[must_use]
    pub fn mount(&self, writable: bool) -> Mount {
        self.mount_as(".", writable)
    }

    /// A [`Mount`] preopening this directory into the guest sandbox as `name`.
    #[must_use]
    pub fn mount_as(&self, name: &str, writable: bool) -> Mount {
        Mount {
            name: name.to_owned(),
            path: self.path().to_path_buf(),
            writable,
        }
    }

    /// Writes `contents` at `relative`, creating parent directories.
    ///
    /// # Panics
    ///
    /// Panics if the file cannot be written.
    pub fn write(&self, relative: &str, contents: impl AsRef<[u8]>) {
        let target = self.path().join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("creating scratch subdirectory");
        }
        fs::write(&target, contents).expect("writing scratch file");
    }

    /// Reads the file at `relative`, if present.
    #[must_use]
    pub fn read(&self, relative: &str) -> Option<Vec<u8>> {
        fs::read(self.path().join(relative)).ok()
    }
}

/// A fresh [`Scratch`] directory.
///
/// # Panics
///
/// Panics if the directory cannot be created.
#[must_use]
pub fn scratch() -> Scratch {
    Scratch(tempfile::Builder::new().prefix("omnia-test-").tempdir().expect("creating scratch dir"))
}
