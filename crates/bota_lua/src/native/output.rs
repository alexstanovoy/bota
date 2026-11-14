//! A requested output replaces its destination only after every write succeeds.
use std::{
    fs::{File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_OUTPUT: AtomicU64 = AtomicU64::new(0);
pub struct AtomicOutput {
    temporary: PathBuf,
    destination: PathBuf,
    file: Option<BufWriter<File>>,
}
impl AtomicOutput {
    pub fn create(destination: &Path) -> io::Result<Self> {
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temporary = parent.join(format!(
            ".script-output-{}-{}",
            std::process::id(),
            NEXT_OUTPUT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        Ok(Self {
            temporary,
            destination: destination.to_owned(),
            file: Some(BufWriter::new(file)),
        })
    }
    pub fn commit(mut self) -> io::Result<()> {
        self.file.as_mut().expect("open output").flush()?;
        drop(self.file.take());
        std::fs::rename(&self.temporary, &self.destination)
    }
    pub fn write_atomic(
        path: &Path,
        write: impl FnOnce(&mut Self) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut output = Self::create(path)?;
        write(&mut output)?;
        output.commit()
    }
}
impl Write for AtomicOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.as_mut().expect("open output").write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().expect("open output").flush()
    }
}
impl Drop for AtomicOutput {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.temporary);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_write_preserves_destination_and_leaves_no_partial_output() {
        let root = std::env::temp_dir().join(format!("atomic-output-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("summary.json");
        std::fs::write(&path, b"previous").unwrap();
        let result = AtomicOutput::write_atomic(&path, |writer| {
            writer.write_all(b"partial")?;
            Err(io::Error::other("injected write failure"))
        });
        assert_eq!(result.unwrap_err().to_string(), "injected write failure");
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn recording_replaces_the_symlink_without_following_it() {
        let root = std::env::temp_dir().join(format!("atomic-link-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let victim = root.join("victim");
        let path = root.join("recording");
        std::fs::write(&victim, b"unchanged").unwrap();
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        AtomicOutput::write_atomic(&path, |writer| writer.write_all(b"recording")).unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"unchanged");
        assert_eq!(std::fs::read(&path).unwrap(), b"recording");
        assert!(!path.is_symlink());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(victim).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
