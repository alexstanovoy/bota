use std::os::fd::AsRawFd;
use std::path::Path;

use rustix::fd::OwnedFd;
use rustix::fs::{self, Dir, FileType, Mode, OFlags, ResolveFlags, Stat};

use crate::{BindingError, Limits, ModuleBank, ModuleName};

const MAX_ENTRIES: usize = 16384;
const MAX_INTERRUPTS: usize = 8;

pub(super) fn load(
    pack: &Path,
    runtime: Option<&Path>,
    limits: Limits,
) -> Result<ModuleBank, BindingError> {
    let mut bank = ModuleBank::new(limits);
    let mut entries = 0;
    Reader::new(pack, "bots", limits)?.walk_root(&mut bank, &mut entries)?;
    if let Some(runtime) = runtime {
        Reader::new(runtime, "game", limits)?.walk_root(&mut bank, &mut entries)?;
    }
    Ok(bank)
}

struct Reader {
    root: OwnedFd,
    proc: OwnedFd,
    limits: Limits,
    namespace: &'static str,
}

#[derive(Debug)]
enum EntryKind { Directory, File }

impl Reader {
    fn new(root: &Path, namespace: &'static str, limits: Limits) -> Result<Self, BindingError> {
        let root = std::fs::canonicalize(root).map_err(|_| source_error(namespace, "", "module root cannot be resolved"))?;
        let root = fs::openat2(
            fs::CWD,
            root,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| source_error(namespace, "", "module root cannot be opened without symbolic links"))?;
        let proc = fs::open(
            "/proc/self/fd",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| source_error(namespace, "", "trusted descriptor directory cannot be opened"))?;
        if fs::fstatfs(&proc).map_err(|_| source_error(namespace, "", "descriptor filesystem metadata cannot be read"))?.f_type != fs::PROC_SUPER_MAGIC {
            return Err(source_error(namespace, "", "descriptor directory is not procfs"));
        }
        Ok(Self { root, proc, limits, namespace })
    }

    fn walk_root(
        &self,
        bank: &mut ModuleBank,
        entries: &mut usize,
    ) -> Result<(), BindingError> {
        self.walk(&self.root, "", 0, bank, entries)
    }

    fn pin(&self, relative: &str) -> Result<OwnedFd, BindingError> {
        // O_PATH classifies without invoking a device's open operation. No descendant mount or symlink is crossed.
        fs::openat2(
            &self.root,
            relative,
            OFlags::PATH | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )
        .map_err(|error| self.error(relative, match error {
            rustix::io::Errno::LOOP => "symbolic links are not allowed",
            rustix::io::Errno::XDEV => "mount crossings are not allowed",
            rustix::io::Errno::NOENT => "module source no longer exists",
            _ => "module source cannot be inspected",
        }))
    }

    fn error(&self, relative: &str, reason: impl Into<String>) -> BindingError {
        source_error(self.namespace, relative, reason)
    }

    fn entry_kind(&self, relative: &str, stat: &Stat) -> Result<EntryKind, BindingError> {
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => Ok(EntryKind::Directory),
            FileType::RegularFile => Ok(EntryKind::File),
            FileType::Symlink => Err(self.error(relative, "symbolic links are not allowed")),
            FileType::Fifo => Err(self.error(relative, "FIFO sources are not allowed")),
            FileType::CharacterDevice => Err(self.error(relative, "character device sources are not allowed")),
            FileType::BlockDevice => Err(self.error(relative, "block device sources are not allowed")),
            FileType::Socket => Err(self.error(relative, "socket sources are not allowed")),
            _ => Err(self.error(relative, "unsupported module source kind")),
        }
    }

    fn walk(
        &self,
        pinned: &OwnedFd,
        relative: &str,
        depth: usize,
        bank: &mut ModuleBank,
        entries: &mut usize,
    ) -> Result<(), BindingError> {
        if depth > ModuleName::MAX_COMPONENTS {
            return Err(self.error(relative, "module directory depth limit exceeded"));
        }
        let readable = fs::openat(
            pinned,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| self.error(relative, "module directory cannot be opened"))?;
        let mut directory = Dir::new(readable).map_err(|_| self.error(relative, "module directory cannot be read"))?;
        while let Some(entry) = directory.read() {
            if *entries >= MAX_ENTRIES {
                return Err(self.error(relative, "module directory entry limit exceeded"));
            }
            *entries += 1;
            let entry = entry.map_err(|_| self.error(relative, "directory entry cannot be read"))?;
            let Ok(name) = entry.file_name().to_str() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let path = if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            };
            if path.len() > ModuleName::MAX_LENGTH + 4 {
                return Err(self.error(&path, "module path limit exceeded"));
            }
            let pinned = self.pin(&path)?;
            let stat = fs::fstat(&pinned).map_err(|_| self.error(&path, "module source metadata cannot be read"))?;
            match self.entry_kind(&path, &stat)? {
                EntryKind::Directory => {
                    self.walk(&pinned, &path, depth + 1, bank, entries)?
                }
                EntryKind::File => {
                    if let Some(module) = path.strip_suffix(".lua") {
                        let _: ModuleName = module
                            .parse()
                            .map_err(|_| self.error(&path, "invalid module name"))?;
                        let source = self.read_regular(&path, &pinned, &stat)?;
                        bank.insert_file(self.namespace, module, &source).map_err(|error| self.error(&path, error.to_string()))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn read_regular(&self, relative: &str, pinned: &OwnedFd, before: &Stat) -> Result<Vec<u8>, BindingError> {
        require_single_link(before).map_err(|error| self.error(relative, error.to_string()))?;
        let size = usize::try_from(before.st_size).map_err(|_| self.error(relative, "module source size cannot be represented"))?;
        if size > self.limits.module_size {
            return Err(self.error(relative, "module source limit exceeded"));
        }
        // Reopen the pinned inode, never its replaceable source name. The O_PATH descriptor stays alive throughout.
        let name = pinned.as_raw_fd().to_string();
        let readable = fs::openat(
            &self.proc,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY,
            Mode::empty(),
        )
        .map_err(|_| self.error(relative, "pinned module source cannot be reopened"))?;
        let reopened = fs::fstat(&readable).map_err(|_| self.error(relative, "reopened source metadata cannot be read"))?;
        require_single_link(&reopened).map_err(|error| self.error(relative, error.to_string()))?;
        if !same_object(before, &reopened)
            || FileType::from_raw_mode(reopened.st_mode) != FileType::RegularFile
        {
            return Err(self.error(relative, "pinned source identity or kind changed"));
        }
        let limit = size
            .checked_add(1)
            .ok_or_else(|| self.error(relative, "module source limit exceeded"))?;
        let source = read_bounded(&readable, limit).map_err(|error| self.error(relative, error.to_string()))?;
        let after = fs::fstat(&readable).map_err(|_| self.error(relative, "read source metadata cannot be read"))?;
        require_single_link(&after).map_err(|error| self.error(relative, error.to_string()))?;
        if source.len() != size || !unchanged(before, &after) {
            return Err(self.error(relative, "module source changed while reading"));
        }
        Ok(source)
    }
}

fn require_single_link(stat: &Stat) -> Result<(), BindingError> {
    if stat.st_nlink > 1 {
        return Err(BindingError::Configuration("multiple hard links are not allowed"));
    }
    Ok(())
}

fn same_object(a: &Stat, b: &Stat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino
}

fn unchanged(a: &Stat, b: &Stat) -> bool {
    same_object(a, b)
        && a.st_size == b.st_size
        && a.st_mtime == b.st_mtime
        && a.st_mtime_nsec == b.st_mtime_nsec
        && a.st_ctime == b.st_ctime
        && a.st_ctime_nsec == b.st_ctime_nsec
}

fn read_bounded(file: &OwnedFd, limit: usize) -> Result<Vec<u8>, BindingError> {
    let mut source = Vec::with_capacity(limit);
    let mut buffer = [0u8; 8192];
    let mut interrupted = 0;
    while source.len() < limit {
        let wanted = buffer.len().min(limit - source.len());
        match rustix::io::read(file, &mut buffer[..wanted]) {
            Ok(0) => break,
            Ok(count) => source.extend_from_slice(&buffer[..count]),
            Err(rustix::io::Errno::INTR) if interrupted < MAX_INTERRUPTS => {
                interrupted += 1;
            }
            Err(_) => return Err(BindingError::Configuration("module source read failed")),
        }
    }
    Ok(source)
}

fn source_error(namespace: &str, relative: &str, reason: impl Into<String>) -> BindingError {
    BindingError::Source { path: format!("{namespace}/{relative}"), reason: reason.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_is_named_and_rejected_using_only_pinned_metadata() {
        let reader = Reader::new(Path::new("/dev"), "bots", Limits::DEFAULT).expect("device directory metadata");
        let pinned = reader.pin("null").expect("O_PATH device metadata");
        let stat = fs::fstat(&pinned).expect("device metadata");
        let error = reader.entry_kind("null", &stat).expect_err("devices never reach a readable open");
        assert_eq!(error.to_string(), r#"module source "bots/null": character device sources are not allowed"#);
    }
}
