use std::{path::PathBuf, process::ExitCode, time::Duration};
use bota_client_core::{Link, Transport, greet};
use bota_lua::{BotSeat, Limits, native::{NativeConfig, NativeTeam, load_modules, play_team, random_keys}};
use bota_proto::{ClientMsg, HeroId, Role, SeatKey, Team};

struct Credential { name: String, key: Option<SeatKey> }
struct Options { scripts: PathBuf, runtime: Option<PathBuf>, address: String, team: Team, hero: HeroId, think_ms: u64 }
fn main() -> ExitCode {
    match run() { Ok(()) => ExitCode::SUCCESS, Err(error) => { eprintln!("{error}"); ExitCode::FAILURE } }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = options()?;
    let credentials = credentials(options.team)?;
    let modules = load_modules(&options.scripts, options.runtime.as_deref(), Limits::DEFAULT)?;
    let mut joined = Vec::new();
    for credential in credentials {
        let mut link = Link::connect(&options.address, Duration::from_secs(30))?;
        let lobby = greet(&mut link, Role::Bot, &credential.name, credential.key)?;
        joined.push((link, lobby.seated().ok_or("server assigned no seat")?));
    }
    let seats = joined.iter().map(|(_, seated)| BotSeat { slot: seated.slot, hero: options.hero }).collect();
    let mut config = NativeConfig::new(options.team, seats, random_keys()?);
    config.think_timeout = Duration::from_millis(options.think_ms);
    let mut worker = NativeTeam::new(config, modules)?;
    for (link, seated) in &mut joined {
        let hero = worker.seats().iter().find(|seat| seat.slot == seated.slot).ok_or("missing selected hero")?.hero;
        link.send(&ClientMsg::PickHero { hero })?;
        link.send(&ClientMsg::SetReady(true))?;
        seated.hero = Some(hero);
    }
    let result = play_team(joined, &mut worker)?;
    println!("match_result seats={} orders={} script_failures={} retired={}", result.outcomes.len(), result.orders_sent, result.script_failures, result.retired);
    Ok(())
}
fn options() -> Result<Options, Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).take(33).collect();
    if args.len() > 32 || args.len() % 2 != 0 { return Err("usage: bota_botscript --scripts DIR [--runtime-scripts DIR] [--address HOST:PORT] [--team radiant|dire] [--hero KEY] [--think-ms N]".into()); }
    let mut options = Options { scripts: PathBuf::new(), runtime: None, address: "127.0.0.1:4455".to_owned(), team: Team::Radiant, hero: HeroId::SHADOW_FIEND, think_ms: 33 };
    for pair in args.as_chunks::<2>().0 {
        match pair[0].to_str().ok_or("invalid option")? {
            "--scripts" => options.scripts = PathBuf::from(&pair[1]), "--runtime-scripts" => options.runtime = Some(PathBuf::from(&pair[1])),
            "--address" => options.address = pair[1].to_str().ok_or("invalid address")?.to_owned(),
            "--team" => options.team = match pair[1].to_str() { Some("radiant" | "0") => Team::Radiant, Some("dire" | "1") => Team::Dire, _ => return Err("invalid team".into()) },
            "--hero" => options.hero = pair[1].to_str().and_then(bota_core::hero_by_key).ok_or("unregistered hero")?,
            "--think-ms" => options.think_ms = pair[1].to_str().ok_or("invalid budget")?.parse()?,
            _ => return Err("unknown option".into()),
        }
    }
    if options.scripts.as_os_str().is_empty() { return Err("--scripts is required".into()); }
    Ok(options)
}
fn credentials(team: Team) -> Result<Vec<Credential>, Box<dyn std::error::Error>> {
    let text = match std::env::var("BOTA_SEATS") { Ok(text) => text, Err(std::env::VarError::NotPresent) => return Ok(vec![Credential { name: "lua".to_owned(), key: None }]), Err(_) => return Err("invalid BOTA_SEATS".into()) };
    if text.len() > 4096 { return Err("too many seat credentials".into()); }
    let mut seats = Vec::new();
    for (index, entry) in text.split(',').enumerate() {
        if index >= 10 { return Err("too many seat credentials".into()); }
        let fields: Vec<_> = entry.split(':').collect();
        let (side, name, kind) = match fields.as_slice() {
            [_, name] => (if index % 2 == 0 { Team::Radiant } else { Team::Dire }, *name, "bot"),
            [_, side, name] => (parse_team(side)?, *name, "bot"), [_, side, name, kind] => (parse_team(side)?, *name, *kind),
            _ => return Err("invalid seat credential format".into()),
        };
        if side != team || kind == "human" { continue; }
        if kind != "bot" { return Err("invalid seat kind".into()); }
        let key = SeatKey::from_hex(fields[0]).ok_or("invalid seat credential")?;
        seats.push(Credential { name: name.to_owned(), key: Some(key) });
    }
    if seats.is_empty() { return Err("no bot credentials for the selected team".into()); }
    Ok(seats)
}
fn parse_team(text: &str) -> Result<Team, &'static str> { match text { "0" => Ok(Team::Radiant), "1" => Ok(Team::Dire), _ => Err("invalid credential team") } }
