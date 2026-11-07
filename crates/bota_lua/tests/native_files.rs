#![cfg(all(feature = "native", target_os = "linux"))]

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bota_lua::{BotSeat, Keys, Limits, LuaTeam, ScriptKind};
use bota_proto::{HeroId, SlotId, Team};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bota_lua_files_{}_{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).expect("private fixture directory");
        Self(root)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove owned fixture");
    }
}

#[test]
fn preloaded_modules_use_distinct_roots_and_need_no_files_during_think() {
    let pack = Directory::new();
    let runtime = Directory::new();
    std::fs::create_dir(pack.path().join("game")).expect("pack subdirectory");
    std::fs::write(pack.path().join("game/shared.lua"), b"return 17").expect("pack module");
    std::fs::write(runtime.path().join("shared.lua"), b"return 29").expect("runtime module");
    let modules =
        bota_lua::native::load_modules(pack.path(), Some(runtime.path()), Limits::DEFAULT)
            .expect("preload");
    std::fs::remove_file(pack.path().join("game/shared.lua")).expect("remove source");
    std::fs::remove_file(runtime.path().join("shared.lua")).expect("remove source");
    let (_, sessions) = support::local(&[Team::Radiant, Team::Dire]);
    let mut lua = LuaTeam::new(
        Team::Radiant,
        &[BotSeat {
            slot: SlotId(0),
            hero: HeroId::SHADOW_FIEND,
        }],
        Limits::DEFAULT,
        Keys {
            hash: [17, 19],
            random: 23,
        },
    )
    .expect("team");
    lua.set_modules(modules).expect("modules");
    lua.add_script(SlotId(0), ScriptKind::Takeover, "preloaded", b"function Think() assert(require('bots/game/shared') == 17) assert(require('game/shared') == 29) assert(dofile('game/shared') == 29) end").expect("script");
    let turn = lua.think(&sessions[0]).expect("think");
    assert!(turn.failures.is_empty(), "{:?}", turn.failures);
}

#[test]
fn source_reader_refuses_symlinks_fifos_and_oversized_regular_files_without_paths() {
    for kind in [0, 1, 2] {
        let pack = Directory::new();
        let outside = Directory::new();
        let file = pack.path().join("module.lua");
        match kind {
            0 => {
                std::fs::write(outside.path().join("secret.lua"), b"return 'outside'")
                    .expect("outside");
                std::os::unix::fs::symlink(outside.path().join("secret.lua"), &file)
                    .expect("symlink");
            }
            1 => rustix::fs::mknodat(
                rustix::fs::CWD,
                &file,
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
                0,
            )
            .expect("fifo"),
            _ => std::fs::write(&file, [b' '; 17]).expect("oversize"),
        }
        let error = bota_lua::native::load_modules(
            pack.path(),
            None,
            Limits {
                module_size: 16,
                ..Limits::DEFAULT
            },
        )
        .expect_err("refused file");
        let message = error.to_string();
        assert!(!message.contains(pack.path().to_str().expect("fixture name")));
        assert!(!message.contains(outside.path().to_str().expect("fixture name")));
        assert!(
            message.contains("not found") || message.contains("limit"),
            "{message}"
        );
    }
}

#[test]
fn source_reader_refuses_a_symlinked_directory_and_excessive_depth() {
    let pack = Directory::new();
    let outside = Directory::new();
    std::fs::write(outside.path().join("module.lua"), b"return 17").expect("outside");
    std::os::unix::fs::symlink(outside.path(), pack.path().join("linked"))
        .expect("directory symlink");
    assert!(bota_lua::native::load_modules(pack.path(), None, Limits::DEFAULT).is_err());
    std::fs::remove_file(pack.path().join("linked")).expect("remove symlink");
    let mut directory = pack.path().to_path_buf();
    for _ in 0..17 {
        directory.push("nested");
        std::fs::create_dir(&directory).expect("nested directory");
    }
    std::fs::write(directory.join("module.lua"), b"return 17").expect("module");
    assert!(
        bota_lua::native::load_modules(pack.path(), None, Limits::DEFAULT)
            .expect_err("depth limit")
            .to_string()
            .contains("limit")
    );
}
