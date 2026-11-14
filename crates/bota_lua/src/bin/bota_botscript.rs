use bota_client_core::{Link, Transport, greet};
use bota_lua::{
    BotSeat, SCRIPT_LIMITS,
    native::{
        HeroSelection, NativeConfig, NativeTeam, load_modules, play_team_recorded, random_keys,
    },
};
use bota_proto::{ClientMsg, HeroId, Role, SeatKey, Team};
use std::{path::PathBuf, process::ExitCode, time::Duration};

struct Credential {
    name: String,
    key: Option<SeatKey>,
}
struct Options {
    scripts: PathBuf,
    runtime: Option<PathBuf>,
    address: String,
    team: Team,
    hero: HeroId,
    think_ms: u64,
    hero_selection: HeroSelection,
    hash_keys: Option<[u64; 2]>,
    rng_seed: Option<u64>,
    summary: Option<PathBuf>,
    replay: Option<PathBuf>,
    recording: Option<PathBuf>,
    seat: u8,
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", diagnostic(error.as_ref()));
            ExitCode::FAILURE
        }
    }
}
fn diagnostic(error: &dyn std::error::Error) -> String {
    error.to_string().escape_debug().to_string()
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os()
        .skip(1)
        .eq([std::ffi::OsString::from("--help")])
    {
        println!(
            "bota_botscript --scripts DIR [--hash-keys A,B --rng-seed N] [--summary-json PATH] [--record-views PATH | --replay-in PATH --seat N] [--selection pack|fixed] [--team radiant|dire] [--address HOST:PORT] [--think-ms N] [--runtime-scripts DIR] [--hero KEY]\nDefault: hash keys and random seed come from OS entropy (printed at startup) and each turn is bounded by fuel and a wall deadline (--think-ms), so two runs of a match can differ.\nSupply both --hash-keys and --rng-seed for a reproducible fuel-only run without wall deadlines: the same pack, binary, keys, seed and game give the same orders. One of them alone changes nothing about the deadline.\nSummary measurement costs about 0.5 microseconds per API call and may change live discard timing; measured and unmeasured strength results are not comparable."
        );
        return Ok(());
    }

    if std::env::args_os()
        .skip(1)
        .eq([std::ffi::OsString::from("--api-catalogue")])
    {
        print!("{}", bota_lua::LuaTeam::api_catalogue()?);
        return Ok(());
    }
    let options = options()?;
    let mut keys = if let (Some(hash), Some(random)) = (options.hash_keys, options.rng_seed) {
        bota_lua::Keys { hash, random }
    } else {
        random_keys()?
    };
    if let Some(hash) = options.hash_keys {
        keys.hash = hash;
    }
    if let Some(random) = options.rng_seed {
        keys.random = random;
    }
    println!(
        "script_keys hash_keys={},{} rng_seed={}",
        keys.hash[0], keys.hash[1], keys.random
    );
    let result = run_match(&options, keys);
    if let Err(error) = &result
        && let Some(path) = &options.summary
    {
        let mut stats = bota_lua::statistics::RunStatistics::default();
        stats.failure(&error.to_string());
        stats.write_atomic(path, keys, serde_json::json!({"error":stats.first_error}))?;
    }
    result
}
fn run_match(options: &Options, keys: bota_lua::Keys) -> Result<(), Box<dyn std::error::Error>> {
    if options.replay.is_some() {
        return run_recorded(options, keys);
    }
    run_live(options, keys)
}
fn run_recorded(options: &Options, keys: bota_lua::Keys) -> Result<(), Box<dyn std::error::Error>> {
    let path = options.replay.as_ref().expect("recorded input");
    let modules = load_modules(&options.scripts, options.runtime.as_deref(), SCRIPT_LIMITS)?;
    let result = bota_lua::native::refeed(
        bota_client_core::ReplayPlayer::load(path)?,
        bota_proto::SlotId(options.seat),
        modules,
        keys,
        options.hero_selection,
        |_, _| {},
    )?;
    if let Some(path) = &options.summary {
        result.statistics.write_atomic(path,keys,serde_json::json!({"winner":format!("{:?}",result.winner),"ticks":result.ticks,"orders":result.orders,"offline":true}))?;
    }
    println!(
        "refeed_result ticks={} orders={} script_failures={}",
        result.ticks, result.orders, result.statistics.script_failures
    );
    Ok(())
}
fn run_live(options: &Options, keys: bota_lua::Keys) -> Result<(), Box<dyn std::error::Error>> {
    let modules = load_modules(&options.scripts, options.runtime.as_deref(), SCRIPT_LIMITS)?;
    let mut joined = join(options)?;
    let seats = joined
        .iter()
        .map(|(_, seated)| BotSeat {
            slot: seated.slot,
            hero: options.hero,
        })
        .collect();
    let mut config = NativeConfig::new(options.team, seats, keys);
    config.hero_selection = options.hero_selection;
    config.statistics = options.summary.is_some();
    config.deterministic = options.hash_keys.is_some() && options.rng_seed.is_some();
    config.think_timeout = Duration::from_millis(options.think_ms);
    let mut worker = NativeTeam::new(config, modules)?;
    for (link, seated) in &mut joined {
        let hero = worker
            .seats()
            .iter()
            .find(|seat| seat.slot == seated.slot)
            .ok_or("missing selected hero")?
            .hero;
        link.send(&ClientMsg::PickHero { hero })?;
        link.send(&ClientMsg::SetReady(true))?;
        seated.hero = Some(hero);
    }
    let mut recording = options
        .recording
        .as_ref()
        .map(|path| bota_lua::native::AtomicOutput::create(path))
        .transpose()?;
    let result = play_team_recorded(
        joined,
        &mut worker,
        recording
            .as_mut()
            .map(|writer| writer as &mut dyn std::io::Write),
    )?;
    if let Some(writer) = recording {
        writer.commit()?;
    }
    if let (Some(path), Some(stats)) = (&options.summary, worker.statistics()) {
        stats.write_atomic(path,keys,serde_json::json!({"retired":result.retired,"orders":result.orders_sent,"seats":result.outcomes.iter().map(|outcome| serde_json::json!({"slot":outcome.slot.0,"winner":format!("{:?}",outcome.winner),"ticks":outcome.ticks})).collect::<Vec<_>>()}))?;
    }
    println!(
        "match_result seats={} orders={} script_failures={} retired={} script_initializations={} script_hooks={} native_hooks={} suppressed_team_errors={} deadline_ticks={}",
        result.outcomes.len(),
        result.orders_sent,
        result.script_failures,
        result.retired,
        result.hook_activity.initializations,
        result.hook_activity.scripted,
        result.hook_activity.native,
        result.hook_activity.suppressed_team_errors,
        worker.deadline_ticks(),
    );
    Ok(())
}
fn join(
    options: &Options,
) -> Result<Vec<(Link, bota_client_core::Seated)>, Box<dyn std::error::Error>> {
    let credentials = credentials(options.team)?;
    let mut joined = Vec::new();
    for credential in credentials {
        let mut link = Link::connect(&options.address, Duration::from_secs(30))?;
        let lobby = greet(&mut link, Role::Bot, &credential.name, credential.key)?;
        joined.push((link, lobby.seated().ok_or("server assigned no seat")?));
    }
    Ok(joined)
}
fn options() -> Result<Options, Box<dyn std::error::Error>> {
    options_from(std::env::args_os().skip(1).take(33).collect())
}
fn options_from(args: Vec<std::ffi::OsString>) -> Result<Options, Box<dyn std::error::Error>> {
    if args.len() > 32 || !args.len().is_multiple_of(2) {
        return Err("usage: bota_botscript --scripts DIR [--runtime-scripts DIR] [--address HOST:PORT] [--team radiant|dire] [--hero KEY] [--selection pack|fixed] [--think-ms N]".into());
    }
    let mut options = defaults();
    for pair in args.as_chunks::<2>().0 {
        if observation_option(&mut options, &pair[0], &pair[1])? {
            continue;
        }
        match pair[0].to_str().ok_or("invalid option")? {
            "--selection" => {
                options.hero_selection = match pair[1].to_str() {
                    Some("fixed") => HeroSelection::Fixed,
                    Some("pack") => HeroSelection::Pack,
                    _ => return Err("invalid hero selection policy".into()),
                }
            }
            "--scripts" => options.scripts = PathBuf::from(&pair[1]),
            "--runtime-scripts" => options.runtime = Some(PathBuf::from(&pair[1])),
            "--address" => options.address = pair[1].to_str().ok_or("invalid address")?.to_owned(),
            "--team" => {
                options.team = match pair[1].to_str() {
                    Some("radiant" | "0") => Team::Radiant,
                    Some("dire" | "1") => Team::Dire,
                    _ => return Err("invalid team".into()),
                }
            }
            "--hero" => {
                options.hero = pair[1]
                    .to_str()
                    .and_then(bota_core::hero_by_key)
                    .ok_or("unregistered hero")?
            }
            "--think-ms" => options.think_ms = pair[1].to_str().ok_or("invalid budget")?.parse()?,
            _ => return Err("unknown option".into()),
        }
    }
    if options.replay.is_some() && options.recording.is_some() {
        return Err("--replay-in and --record-views are mutually exclusive".into());
    }
    if options.replay.is_some() && (options.hash_keys.is_none() || options.rng_seed.is_none()) {
        return Err("--replay-in requires --hash-keys and --rng-seed".into());
    }
    if options.replay.is_some() && options.summary.is_none() {
        return Err("--replay-in requires --summary-json".into());
    }
    if options.scripts.as_os_str().is_empty() {
        return Err("--scripts is required".into());
    }
    Ok(options)
}
fn defaults() -> Options {
    Options {
        scripts: PathBuf::new(),
        runtime: None,
        address: "127.0.0.1:4455".to_owned(),
        team: Team::Radiant,
        hero: HeroId::SHADOW_FIEND,
        think_ms: 33,
        hero_selection: HeroSelection::Pack,
        hash_keys: None,
        rng_seed: None,
        summary: None,
        replay: None,
        recording: None,
        seat: 0,
    }
}
fn observation_option(
    options: &mut Options,
    key: &std::ffi::OsStr,
    value: &std::ffi::OsStr,
) -> Result<bool, Box<dyn std::error::Error>> {
    let pair = [key, value];
    match key.to_str().ok_or("invalid option")? {
        "--hash-keys" => {
            let text = pair[1].to_str().ok_or("invalid hash keys")?;
            let (a, b) = text.split_once(',').ok_or("hash keys must be A,B")?;
            options.hash_keys = Some([a.parse()?, b.parse()?]);
        }
        "--rng-seed" => {
            options.rng_seed = Some(pair[1].to_str().ok_or("invalid RNG seed")?.parse()?)
        }
        "--summary-json" => options.summary = Some(PathBuf::from(&pair[1])),
        "--record-views" => options.recording = Some(PathBuf::from(&pair[1])),
        "--replay-in" => options.replay = Some(PathBuf::from(&pair[1])),
        "--seat" => {
            options.seat = pair[1].to_str().ok_or("invalid seat")?.parse()?;
            if options.seat >= 10 {
                return Err("invalid seat".into());
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}
fn credentials(team: Team) -> Result<Vec<Credential>, Box<dyn std::error::Error>> {
    let text = match std::env::var("BOTA_SEATS") {
        Ok(text) => text,
        Err(std::env::VarError::NotPresent) => {
            return Ok(vec![Credential {
                name: "lua".to_owned(),
                key: None,
            }]);
        }
        Err(_) => return Err("invalid BOTA_SEATS".into()),
    };
    if text.len() > 4096 {
        return Err("too many seat credentials".into());
    }
    let mut seats = Vec::new();
    for (index, entry) in text.split(',').enumerate() {
        if index >= 10 {
            return Err("too many seat credentials".into());
        }
        let fields: Vec<_> = entry.split(':').collect();
        let (side, name, kind) = match fields.as_slice() {
            [_, name] => (
                if index % 2 == 0 {
                    Team::Radiant
                } else {
                    Team::Dire
                },
                *name,
                "bot",
            ),
            [_, side, name] => (parse_team(side)?, *name, "bot"),
            [_, side, name, kind] => (parse_team(side)?, *name, *kind),
            _ => return Err("invalid seat credential format".into()),
        };
        if side != team || kind == "human" {
            continue;
        }
        if kind != "bot" {
            return Err("invalid seat kind".into());
        }
        let key = SeatKey::from_hex(fields[0]).ok_or("invalid seat credential")?;
        seats.push(Credential {
            name: name.to_owned(),
            key: Some(key),
        });
    }
    if seats.is_empty() {
        return Err("no bot credentials for the selected team".into());
    }
    Ok(seats)
}
fn parse_team(text: &str) -> Result<Team, &'static str> {
    match text {
        "0" => Ok(Team::Radiant),
        "1" => Ok(Team::Dire),
        _ => Err("invalid credential team"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bota_lua::{Keys, ModuleBank, native::NativeError};

    #[test]
    fn replay_options_preserve_exact_keys_seed_and_output_path() {
        let options = options_from(
            [
                "--scripts",
                "pack",
                "--hash-keys",
                "18446744073709551615,0",
                "--rng-seed",
                "17",
                "--summary-json",
                "out.json",
                "--replay-in",
                "match.brp",
                "--seat",
                "1",
            ]
            .into_iter()
            .map(Into::into)
            .collect(),
        )
        .unwrap();
        assert_eq!(options.hash_keys, Some([u64::MAX, 0]));
        assert_eq!(options.rng_seed, Some(17));
        assert_eq!(options.seat, 1);
        assert_eq!(options.summary, Some(PathBuf::from("out.json")));
        assert!(
            options_from(
                ["--scripts", "pack", "--replay-in", "match.brp"]
                    .into_iter()
                    .map(Into::into)
                    .collect()
            )
            .is_err()
        );
    }
    #[test]
    fn recording_and_refeeding_are_explicitly_mutually_exclusive() {
        let result = options_from(
            [
                "--scripts",
                "pack",
                "--replay-in",
                "in.brp",
                "--record-views",
                "out.brp",
            ]
            .into_iter()
            .map(Into::into)
            .collect(),
        );
        assert_eq!(
            result.err().unwrap().to_string(),
            "--replay-in and --record-views are mutually exclusive"
        );
    }
    #[test]
    fn startup_diagnostics_escape_script_control_characters() {
        let mut modules = ModuleBank::new(SCRIPT_LIMITS);
        modules
            .insert(
                "bots/hero_selection",
                br#"function Think() error("bad\27[31mcolour\7\r\nnext") end"#,
            )
            .expect("source");
        let config = NativeConfig::new(
            Team::Radiant,
            vec![BotSeat {
                slot: bota_proto::SlotId(0),
                hero: HeroId::SHADOW_FIEND,
            }],
            Keys {
                hash: [17, 19],
                random: 23,
            },
        );
        let error = NativeTeam::new(config, modules)
            .err()
            .expect("the script rejects startup");
        let raw = error.to_string();
        assert!(raw.contains('\x1b'), "the script supplied an escape byte");
        assert!(raw.contains('\x07'), "the script supplied a bell byte");
        let rendered = diagnostic(&error);
        assert!(
            !rendered.chars().any(char::is_control),
            "startup diagnostics must not emit raw control characters"
        );
        for escaped in ["\\u{1b}", "\\u{7}", "\\r", "\\n"] {
            assert!(
                rendered.contains(escaped),
                "the diagnostic preserves the escaped control spelling"
            );
        }
    }

    #[test]
    fn startup_diagnostics_preserve_ordinary_text_and_unicode() {
        let error = NativeError::Setup("plain café λ".to_owned());
        assert_eq!(diagnostic(&error), "plain café λ");
    }
}
