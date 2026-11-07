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
    Reader::new(pack, limits)?.walk_root("bots", &mut bank, &mut entries)?;
    if let Some(runtime) = runtime {
        Reader::new(runtime, limits)?.walk_root("game", &mut bank, &mut entries)?;
    }
    Ok(bank)
}

struct Reader {
    root: OwnedFd,
    proc: OwnedFd,
    limits: Limits,
}

impl Reader {
    fn new(root: &Path, limits: Limits) -> Result<Self, BindingError> {
        let root = std::fs::canonicalize(root).map_err(|_| not_found())?;
        let root = fs::openat2(
            fs::CWD,
            root,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| not_found())?;
        let proc = fs::open(
            "/proc/self/fd",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| not_found())?;
        if fs::fstatfs(&proc).map_err(|_| not_found())?.f_type != fs::PROC_SUPER_MAGIC {
            return Err(not_found());
        }
        Ok(Self { root, proc, limits })
    }

    fn walk_root(
        &self,
        namespace: &str,
        bank: &mut ModuleBank,
        entries: &mut usize,
    ) -> Result<(), BindingError> {
        self.walk(&self.root, "", 0, namespace, bank, entries)
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
        .map_err(|_| not_found())
    }

    fn walk(
        &self,
        pinned: &OwnedFd,
        relative: &str,
        depth: usize,
        namespace: &str,
        bank: &mut ModuleBank,
        entries: &mut usize,
    ) -> Result<(), BindingError> {
        if depth > ModuleName::MAX_COMPONENTS {
            return Err(BindingError::Configuration(
                "module directory depth limit exceeded",
            ));
        }
        let readable = fs::openat(
            pinned,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| not_found())?;
        let mut directory = Dir::new(readable).map_err(|_| not_found())?;
        while let Some(entry) = directory.read() {
            if *entries >= MAX_ENTRIES {
                return Err(BindingError::Configuration(
                    "module directory entry limit exceeded",
                ));
            }
            *entries += 1;
            let entry = entry.map_err(|_| not_found())?;
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
                return Err(BindingError::Configuration("module path limit exceeded"));
            }
            let pinned = self.pin(&path)?;
            let stat = fs::fstat(&pinned).map_err(|_| not_found())?;
            match FileType::from_raw_mode(stat.st_mode) {
                FileType::Directory => {
                    self.walk(&pinned, &path, depth + 1, namespace, bank, entries)?
                }
                FileType::RegularFile => {
                    if let Some(module) = path.strip_suffix(".lua") {
                        let _: ModuleName = module
                            .parse()
                            .map_err(|_| BindingError::Configuration("invalid module name"))?;
                        let source = self.read_regular(&pinned, &stat)?;
                        bank.insert_file(namespace, module, &source)?;
                    }
                }
                _ => return Err(not_found()),
            }
        }
        Ok(())
    }

    fn read_regular(&self, pinned: &OwnedFd, before: &Stat) -> Result<Vec<u8>, BindingError> {
        require_single_link(before)?;
        let size = usize::try_from(before.st_size).map_err(|_| not_found())?;
        if size > self.limits.module_size {
            return Err(BindingError::Configuration("module source limit exceeded"));
        }
        // Reopen the pinned inode, never its replaceable source name. The O_PATH descriptor stays alive throughout.
        let name = pinned.as_raw_fd().to_string();
        let readable = fs::openat(
            &self.proc,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY,
            Mode::empty(),
        )
        .map_err(|_| not_found())?;
        let reopened = fs::fstat(&readable).map_err(|_| not_found())?;
        require_single_link(&reopened)?;
        if !same_object(before, &reopened)
            || FileType::from_raw_mode(reopened.st_mode) != FileType::RegularFile
        {
            return Err(not_found());
        }
        let limit = size
            .checked_add(1)
            .ok_or(BindingError::Configuration("module source limit exceeded"))?;
        let source = read_bounded(&readable, limit)?;
        let after = fs::fstat(&readable).map_err(|_| not_found())?;
        require_single_link(&after)?;
        if source.len() != size || !unchanged(before, &after) {
            return Err(not_found());
        }
        Ok(source)
    }
}

fn require_single_link(stat: &Stat) -> Result<(), BindingError> {
    if stat.st_nlink > 1 {
        return Err(BindingError::Configuration("module source has multiple hard links"));
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
            Err(_) => return Err(not_found()),
        }
    }
    Ok(source)
}

fn not_found() -> BindingError {
    BindingError::Configuration("module source not found")
}
