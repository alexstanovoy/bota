//! Dota's cheat commands, typed into the chat (`-gold 500`) or the console
//! (`dota_dev player_givegold 500`), turned into the cheat orders they ask
//! for. Names and meanings are Dota's (dota2.fandom.com/wiki/Cheats); the
//! server honours them only in a match started with cheats on.

use bota_proto::{Cheat, CreepSide, HeroId, ItemId, Order, Vec2};

/// What a command needs of the client beyond its own words.
pub trait CommandContext {
    /// The item a name stands for: its Dota key (`item_blink`, or `blink`
    /// as Dota's chat takes it), or its id.
    fn item(&self, word: &str) -> Option<ItemId>;
    /// The hero a name stands for, the same way (`npc_dota_hero_pudge`, or
    /// `pudge`).
    fn hero(&self, word: &str) -> Option<HeroId>;
    /// The spot on the map under the cursor, for what Dota puts there
    /// (`-createhero`, `-teleport`).
    fn cursor(&self) -> Option<Vec2>;
}

/// The names this crate knows: the rules' items and heroes by Dota key or
/// id; no cursor.
pub struct Plain;

impl CommandContext for Plain {
    fn item(&self, word: &str) -> Option<ItemId> {
        if let Ok(id) = word.parse::<u16>() {
            return bota_core::item_def(ItemId(id)).map(|_| ItemId(id));
        }
        by_key(word, "item_", bota_core::item_by_key)
    }

    fn hero(&self, word: &str) -> Option<HeroId> {
        if let Ok(id) = word.parse::<u16>() {
            return bota_core::hero_def(HeroId(id)).map(|_| HeroId(id));
        }
        by_key(word, "npc_dota_hero_", bota_core::hero_by_key)
    }

    fn cursor(&self) -> Option<Vec2> {
        None
    }
}

/// What a word names as a Dota key, in any case, with or without the
/// prefix every key of its kind starts with.
fn by_key<T>(word: &str, prefix: &str, lookup: fn(&str) -> Option<T>) -> Option<T> {
    let word = word.to_ascii_lowercase();
    lookup(&word).or_else(|| lookup(&format!("{prefix}{word}")))
}

/// Why a typed line is no command; the player's own word is kept as data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    /// Nothing was typed.
    Empty,
    /// A switch was given a word that is neither 1 nor 0.
    NotOnOff(String),
    /// `item` without the item.
    NoItemNamed,
    /// No item is called so.
    UnknownItem(String),
    /// `createhero` without the hero.
    NoHeroNamed,
    /// No hero is called so.
    UnknownHero(String),
    /// A hero's side was given as neither ally nor enemy.
    NotAllyOrEnemy(String),
    /// The command wants the spot under the cursor and there is none.
    NeedsCursor(CursorUse),
    /// A creep side that is none of radiant, dire, neutral or all.
    NotCreepSide(String),
    /// No command is called so.
    UnknownCommand(String),
    /// A count or amount that is no number.
    NotANumber(String),
}

/// What a command takes the cursor's spot for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorUse {
    CreateHero,
    Teleport,
}

/// Turns one typed line into an order, with this crate's own names. A
/// leading dash is allowed and ignored, so `-gold 500` and `gold 500` ask
/// the same.
pub fn parse_console(line: &str) -> Result<Order, CommandError> {
    parse_command(line, &Plain).map(|cheat| Order::Cheat { cheat })
}

/// Whether a line said in the chat is a cheat command rather than words:
/// a dash, then the name of one (Dota's chat takes those as commands, and
/// says anything else).
pub fn is_chat_command(line: &str) -> bool {
    let Some(rest) = line.trim().strip_prefix('-') else {
        return false;
    };
    let word = rest.split_whitespace().next().unwrap_or("");
    CHAT_COMMANDS.contains(&word.to_ascii_lowercase().as_str())
}

/// The chat commands, as Dota names them.
pub const CHAT_COMMANDS: [&str; 22] = [
    "gold",
    "lvlup",
    "levelbots",
    "wtf",
    "unwtf",
    "refresh",
    "respawn",
    "item",
    "createhero",
    "spawncreeps",
    "disablecreepspawn",
    "enablecreepspawn",
    "spawnneutrals",
    "killcreeps",
    "startgame",
    "allvision",
    "normalvision",
    "teleport",
    // The forms this client always took.
    "levelup",
    "level",
    "invulnerable",
    "vulnerable",
];

/// Turns a typed command into the cheat it asks for: Dota's chat form with
/// or without its dash, or its console form.
pub fn parse_command(line: &str, cx: &dyn CommandContext) -> Result<Cheat, CommandError> {
    let line = line.trim();
    let line = line.strip_prefix('-').unwrap_or(line);
    let mut words = line.split_whitespace();
    let Some(first) = words.next() else {
        return Err(CommandError::Empty);
    };
    let first = first.to_ascii_lowercase();
    // `dota_dev hero_level 3` reads as its second word.
    let command = if first == "dota_dev" {
        words
            .next()
            .map(str::to_ascii_lowercase)
            .unwrap_or_default()
    } else {
        first
    };
    let rest: Vec<&str> = words.collect();
    let arg = rest.first().copied();
    let on = |word: Option<&str>| match word {
        None | Some("1") => Ok(true),
        Some("0") => Ok(false),
        Some(other) => Err(CommandError::NotOnOff(other.to_string())),
    };
    let cheat = match command.as_str() {
        "gold" | "player_givegold" => Cheat::Gold {
            amount: number(arg, 1000)?,
        },
        "lvlup" | "levelup" | "level" | "hero_level" => Cheat::Levels {
            count: number(arg, 1)?,
        },
        "levelbots" | "dota_bot_give_level" => Cheat::LevelBots {
            count: number(arg, 1)?,
        },
        "wtf" => Cheat::Wtf { on: true },
        "unwtf" => Cheat::Wtf { on: false },
        "dota_ability_debug" => Cheat::Wtf { on: on(arg)? },
        "refresh" | "hero_refresh" => Cheat::Refresh,
        "respawn" | "hero_respawn" => Cheat::Respawn,
        "item" | "dota_create_item" => {
            let name = arg.ok_or(CommandError::NoItemNamed)?;
            Cheat::Item {
                item: cx
                    .item(name)
                    .ok_or_else(|| CommandError::UnknownItem(name.to_string()))?,
            }
        }
        "createhero" | "dota_create_unit" => {
            let name = arg.ok_or(CommandError::NoHeroNamed)?;
            let hero = cx
                .hero(name)
                .ok_or_else(|| CommandError::UnknownHero(name.to_string()))?;
            let enemy = match rest.get(1).copied() {
                None | Some("ally") => false,
                Some("enemy") => true,
                Some(other) => return Err(CommandError::NotAllyOrEnemy(other.to_string())),
            };
            let at = cx
                .cursor()
                .ok_or(CommandError::NeedsCursor(CursorUse::CreateHero))?;
            Cheat::CreateHero { hero, enemy, at }
        }
        "spawncreeps" | "dota_spawn_creeps" => Cheat::SpawnCreeps,
        "disablecreepspawn" => Cheat::CreepSpawning { on: false },
        "enablecreepspawn" => Cheat::CreepSpawning { on: true },
        "dota_creeps_no_spawning" => Cheat::CreepSpawning { on: !on(arg)? },
        "spawnneutrals" | "dota_spawn_neutrals" => Cheat::SpawnNeutrals,
        "killcreeps" | "dota_kill_creeps" => Cheat::KillCreeps {
            which: match arg.map(str::to_ascii_lowercase).as_deref() {
                None | Some("all") => CreepSide::All,
                Some("radiant") => CreepSide::Radiant,
                Some("dire") => CreepSide::Dire,
                Some("neutral") => CreepSide::Neutral,
                Some(other) => return Err(CommandError::NotCreepSide(other.to_string())),
            },
        },
        "startgame" | "forcegamestart" => Cheat::StartGame,
        "allvision" | "dota_all_vision_enable" => Cheat::AllVision { on: true },
        "normalvision" | "dota_all_vision_disable" => Cheat::AllVision { on: false },
        "dota_all_vision" => Cheat::AllVision { on: on(arg)? },
        "teleport" | "hero_teleport" => Cheat::Teleport {
            at: cx
                .cursor()
                .ok_or(CommandError::NeedsCursor(CursorUse::Teleport))?,
        },
        "invulnerable" => Cheat::Invulnerable { on: true },
        "vulnerable" => Cheat::Invulnerable { on: false },
        other => return Err(CommandError::UnknownCommand(other.to_string())),
    };
    Ok(cheat)
}

/// The number typed after a command, or the usual one when none was.
fn number<T: std::str::FromStr>(word: Option<&str>, usual: T) -> Result<T, CommandError> {
    match word {
        None => Ok(usual),
        Some(word) => word
            .parse()
            .map_err(|_| CommandError::NotANumber(word.to_string())),
    }
}
