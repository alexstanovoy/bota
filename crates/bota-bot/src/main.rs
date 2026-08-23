//! Command line entry point of the second bot.

use std::path::PathBuf;

use bota_bot::{
    Adam, Chair, DEEDS, Dice, FirstAllowed, LESSONS, Learned, Lesson, Mind, Model, NUMBERS,
    Nothing, Plan, Role, School, Tribe, Yard, crowd_from, first_crowd, gather, learn_from, measure,
    play, report_card, teach_a_lesson,
};
use bota_proto::HeroId;
use clap::{Parser, Subcommand};

/// A bot that decides by naming one of a fixed list of deeds.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    /// What to do. Playing, when nothing is said.
    #[command(subcommand)]
    doing: Option<Doing>,
    #[command(flatten)]
    playing: Playing,
}

/// The things it can be asked to do.
#[derive(Subcommand, Debug)]
enum Doing {
    /// Join a server and play one match.
    Play(Playing),
    /// Say what the contract between the game and a model is.
    Shape,
    /// Write out a model with weights drawn at random.
    Fresh(Fresh),
    /// Breed a crowd of models through the stages a plan names.
    Train(Training),
    /// Teach one lesson by gradient, for comparing against.
    Descend(Descending),
    /// Say what a model is worth at a lesson, on the matches nothing trains on.
    Judge(Judging),
    /// Put two models against each other in whole matches.
    Duel(Duelling),
}

/// Two models against each other.
#[derive(clap::Args, Debug)]
struct Duelling {
    /// Play matches over a socket instead of in this process.
    #[arg(long)]
    on_the_wire: bool,
    /// One of them.
    #[arg(long, value_name = "FILE")]
    one: PathBuf,
    /// The other.
    #[arg(long, value_name = "FILE")]
    other: PathBuf,
    /// Matches to play. Each is played twice, once from either side.
    #[arg(long, default_value_t = 4)]
    matches: usize,
    /// Ticks a match runs before it is called off.
    #[arg(long, default_value_t = 36000)]
    limit: u32,
    /// What the seats are there to do, one to five.
    #[arg(long, default_value_t = 2)]
    role: u8,
    /// How many matches run at once.
    #[arg(long, default_value_t = 8)]
    lanes: usize,
    /// Where the matches are drawn from.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// The server to run. The one built beside this, when nothing is said.
    #[arg(long, value_name = "PATH")]
    server: Option<PathBuf>,
}

/// What one model did over a set of matches.
#[derive(Clone, Copy, Debug, Default)]
struct Tally {
    won: u32,
    kills: u32,
    deaths: u32,
    last_hits: u32,
    denies: u32,
    level: u32,
    played: u32,
    ended: u32,
}

impl Tally {
    /// Adds one seat's match to the tally.
    fn add(&mut self, out: &bota_bot::Outcome) {
        self.played += 1;
        if out.winner.is_some() {
            self.ended += 1;
        }
        if let (Some(winner), Some(team)) = (out.winner, out.team)
            && winner == team
        {
            self.won += 1;
        }
        if let Some(row) = out.mine.as_ref() {
            self.kills += u32::from(row.kills);
            self.deaths += u32::from(row.deaths);
            self.last_hits += u32::from(row.last_hits);
            self.denies += u32::from(row.denies);
            self.level += u32::from(row.level);
        }
    }

    /// The line it prints, per match.
    fn line(&self, name: &str) -> String {
        let over = self.played.max(1) as f32;
        format!(
            "{name:>10}: won {}, {:.1} kills, {:.1} deaths, {:.1} last hits, {:.1} denies, level {:.1}",
            self.won,
            self.kills as f32 / over,
            self.deaths as f32 / over,
            self.last_hits as f32 / over,
            self.denies as f32 / over,
            self.level as f32 / over,
        )
    }
}

/// Plays two models against each other, each seed twice with the sides swapped.
///
/// Swapped because the two sides of a map are not the same to play, and a
/// result read off one side is a result about the map.
fn duel(asked: Duelling) -> std::io::Result<()> {
    let Some(role) = Role::of(asked.role) else {
        return Err(std::io::Error::other("roles are numbered one to five"));
    };
    let standing = Yard::default();
    let yard = Yard {
        server: asked.server.unwrap_or(standing.server),
        builtin: !asked.on_the_wire,
        ..standing
    };
    let load = |path: &PathBuf| -> std::io::Result<Vec<f32>> {
        Model::from_file(path, 1)
            .and_then(|model| model.pour())
            .map_err(std::io::Error::other)
    };
    let bodies = [load(&asked.one)?, load(&asked.other)?];
    let names = [
        asked.one.file_stem().unwrap_or_default().to_string_lossy(),
        asked
            .other
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy(),
    ];
    println!(
        "{} against {}, {} matches from either side, up to {} ticks each",
        names[0], names[1], asked.matches, asked.limit
    );

    let mut dice = Dice::from_seed(asked.seed);
    let seeds: Vec<u64> = (0..asked.matches).map(|_| dice.next_u64()).collect();
    // Each seed twice, with which model sits first swapped the second time.
    let jobs: Vec<(u64, bool)> = seeds
        .iter()
        .flat_map(|seed| [(*seed, false), (*seed, true)])
        .collect();

    let mut tally = [Tally::default(), Tally::default()];
    for batch in jobs.chunks(asked.lanes.max(1)) {
        let played: Vec<std::io::Result<(bool, bota_bot::Outcome, bota_bot::Outcome)>> =
            std::thread::scope(|scope| {
                let running: Vec<_> = batch
                    .iter()
                    .map(|(seed, swapped)| {
                        let (seed, swapped) = (*seed, *swapped);
                        let yard = &yard;
                        let bodies = &bodies;
                        scope.spawn(move || {
                            let hatch = |body: &Vec<f32>| -> std::io::Result<Learned> {
                                let model = Model::fresh(1).map_err(std::io::Error::other)?;
                                model.soak(body).map_err(std::io::Error::other)?;
                                Ok(Learned::new(model))
                            };
                            let first = usize::from(swapped);
                            let mut here = hatch(&bodies[first])?;
                            let mut there = hatch(&bodies[1 - first])?;
                            let chair = |name: &str| Chair {
                                addr: String::new(),
                                name: name.to_string(),
                                hero: yard.hero,
                                limit: Some(asked.limit),
                                role,
                                lesson: Lesson::GrowRich,
                                until: None,
                            };
                            let (mine, theirs) = yard.play_a_match(
                                seed,
                                &mut here,
                                &mut there,
                                &chair("here"),
                                &chair("there"),
                            )?;
                            Ok((swapped, mine, theirs))
                        })
                    })
                    .collect();
                running
                    .into_iter()
                    .map(|one| {
                        one.join()
                            .unwrap_or_else(|_| Err(std::io::Error::other("a match gave up")))
                    })
                    .collect()
            });
        for outcome in played {
            let (swapped, here, there) = outcome?;
            let first = usize::from(swapped);
            tally[first].add(&here);
            tally[1 - first].add(&there);
        }
    }
    let ended = tally[0].ended;
    println!("{}", tally[0].line(&names[0]));
    println!("{}", tally[1].line(&names[1]));
    println!(
        "{ended} of {} matches reached an ancient; the rest were called off at {} ticks",
        tally[0].played, asked.limit
    );
    Ok(())
}

/// Weighing a model up.
#[derive(clap::Args, Debug)]
struct Judging {
    /// Play matches over a socket instead of in this process.
    #[arg(long)]
    on_the_wire: bool,
    /// Which lesson, one to eight.
    #[arg(long, default_value_t = 8)]
    lesson: u8,
    /// What the seats are there to do, one to five.
    #[arg(long, default_value_t = 2)]
    role: u8,
    /// How many matches run at once.
    #[arg(long, default_value_t = 12)]
    lanes: usize,
    /// The model to weigh.
    #[arg(long, value_name = "FILE")]
    weights: Option<PathBuf>,
    /// The server to run. The one built beside this, when nothing is said.
    #[arg(long, value_name = "PATH")]
    server: Option<PathBuf>,
}

/// Says what a model is worth at a lesson, on the reporting matches.
///
/// The same matches whatever the model and however it was taught, so that two
/// ways of teaching can be held against each other.
fn judge(asked: Judging) -> std::io::Result<()> {
    let Some(lesson) = Lesson::of(asked.lesson) else {
        return Err(std::io::Error::other(format!(
            "lessons are numbered one to {}",
            bota_bot::LESSONS
        )));
    };
    let Some(role) = Role::of(asked.role) else {
        return Err(std::io::Error::other("roles are numbered one to five"));
    };
    let standing = Yard::default();
    let tribe = Tribe {
        yard: Yard {
            server: asked.server.unwrap_or(standing.server),
            builtin: !asked.on_the_wire,
            ..standing
        },
        role,
        lanes: asked.lanes,
        ..Tribe::new(1, 1)
    };
    let weights = asked.weights.unwrap_or_else(Model::path);
    let body = Model::from_file(&weights, 1)
        .and_then(|model| model.pour())
        .map_err(std::io::Error::other)?;
    let card = report_card(&tribe, &body)?;
    println!(
        "{}, over {} matches it never trained on",
        weights.display(),
        bota_bot::REPORTED_ON
    );
    for line in card.lines() {
        println!("  {line}");
    }
    let _ = lesson;
    Ok(())
}

/// Following a plan.
#[derive(clap::Args, Debug)]
struct Training {
    /// The plan to follow.
    #[arg(value_name = "FILE")]
    plan: PathBuf,
    /// Where the best of the crowd is kept. A file already there is what the
    /// run continues from; the standing weights file, when nothing is said.
    #[arg(long, value_name = "FILE")]
    weights: Option<PathBuf>,
}

/// Breeds a crowd through the stages of a plan, best first.
fn train(asked: Training) -> std::io::Result<()> {
    let plan = Plan::read(&asked.plan).map_err(std::io::Error::other)?;
    let terms = plan.terms().map_err(std::io::Error::other)?;
    let weights = asked.weights.unwrap_or_else(Model::path);
    let first = terms.first().expect("a plan with a stage");
    println!(
        "following {}: {} stages, played by {}, kept in {}",
        asked.plan.display(),
        terms.len(),
        first.tribe.yard.server.display(),
        weights.display()
    );
    let mut crowd = if weights.exists() {
        let body = Model::from_file(&weights, 1)
            .and_then(|model| model.pour())
            .map_err(std::io::Error::other)?;
        println!("continuing from {}", weights.display());
        crowd_from(&first.tribe, body)
    } else {
        println!("no weights at {}, starting fresh", weights.display());
        first_crowd(&first.tribe).map_err(std::io::Error::other)?
    };
    for (at, term) in terms.iter().enumerate() {
        let tribe = &term.tribe;
        println!(
            "
stage {} of {} — {}, {} ticks a match, scored in {}",
            at + 1,
            terms.len(),
            term.rung.name,
            term.rung.ticks,
            term.rung.scored_in
        );
        println!(
            "  {} models, {} matches each, {} generations, keeping {}, {} matches at once",
            tribe.folk, tribe.trials, tribe.lives, tribe.keep, tribe.lanes
        );
        crowd = teach_a_lesson(tribe, crowd, &term.rung, |life| {
            // Matches that never finished are said out loud. A generation
            // quietly judged on half its matches is a generation judged on
            // luck, and a run that says nothing about it looks like one that
            // went well.
            let lost = if life.failed == 0 {
                String::new()
            } else {
                format!(", {} matches lost", life.failed)
            };
            println!(
                "  generation {}: best {:.1}, middling {:.1}{lost}",
                life.number, life.best, life.middling
            );
        })?;
        // Written now rather than at the end of the plan: a stage of the last
        // rung is hours, and losing five learned stages to whatever goes wrong
        // on the sixth is losing them for nothing.
        keep_the_best(&crowd, &weights)?;
        println!(
            "  learned, and the best of the crowd kept in {}",
            weights.display()
        );
    }
    // One match, run to the longest lesson's clock, scored by every lesson at
    // once: one card about one game rather than a number from each of seven.
    let last = terms.last().expect("a plan with a stage");
    println!(
        "
the best of them, over {} matches it never trained on:",
        bota_bot::REPORTED_ON
    );
    for line in report_card(&last.tribe, &crowd[0])?.lines() {
        println!("  {line}");
    }
    println!(
        "
the best of them is in {}",
        weights.display()
    );
    Ok(())
}

/// Writes the head of the crowd out.
fn keep_the_best(crowd: &[Vec<f32>], weights: &std::path::Path) -> std::io::Result<()> {
    let Some(body) = crowd.first() else {
        return Err(std::io::Error::other("an empty crowd has no best"));
    };
    let best = Model::fresh(1).map_err(std::io::Error::other)?;
    best.soak(body).map_err(std::io::Error::other)?;
    best.save(weights).map_err(std::io::Error::other)
}

/// Teaching one lesson by gradient.
#[derive(clap::Args, Debug)]
struct Descending {
    /// Play matches over a socket instead of in this process.
    #[arg(long)]
    on_the_wire: bool,
    /// Which lesson, one to eight: stock up, find the lane, hold it, meet the
    /// wave, work the lane, take the towers, grow rich, grow strong. Every one
    /// in turn, when nothing is said.
    #[arg(long)]
    lesson: Option<u8>,
    /// What the seats are there to do, one to five.
    #[arg(long, default_value_t = 2)]
    role: u8,
    /// Rounds to run.
    #[arg(long, default_value_t = 20)]
    rounds: u32,
    /// Matches a round.
    #[arg(long, default_value_t = 8)]
    matches: usize,
    /// Matches played at once.
    #[arg(long, default_value_t = 8)]
    lanes: usize,
    /// How loosely it chooses while learning.
    #[arg(long, default_value_t = 1.0)]
    heat: f32,
    /// How far a step goes.
    #[arg(long, default_value_t = 3e-4)]
    rate: f32,
    /// Decisions added up before the weights move.
    #[arg(long, default_value_t = 64)]
    batch: usize,
    /// The most decisions of one round the weights are moved by. A round
    /// usually plays more than this; the rest are thrown away.
    #[arg(long, default_value_t = 8000)]
    frames: usize,
    /// Rounds between measuring.
    #[arg(long, default_value_t = 5)]
    measure_every: u32,
    /// Where the run is seeded from.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Where the model is kept.
    #[arg(long, value_name = "FILE")]
    weights: Option<PathBuf>,
    /// The server to run. The one built beside this, when nothing is said.
    #[arg(long, value_name = "PATH")]
    server: Option<PathBuf>,
}

/// Joining a server and playing.
#[derive(clap::Args, Debug)]
struct Playing {
    /// Where the server listens.
    #[arg(long, default_value = "127.0.0.1:4455")]
    addr: String,
    /// What the lobby shows.
    #[arg(long, default_value = "bot")]
    name: String,
    /// Which hero to ask for.
    #[arg(long, default_value_t = 0)]
    hero: u16,
    /// Leave after this many ticks.
    #[arg(long, value_name = "TICKS")]
    limit: Option<u32>,
    /// Which model to play by. The kept one, when nothing is said.
    #[arg(long, value_name = "FILE")]
    weights: Option<PathBuf>,
    /// Play by the first thing it is allowed rather than by a model, which is
    /// the floor anything trained has to clear.
    #[arg(long)]
    floor: bool,
    /// Do nothing at all, for seeing what a match looks like with a seat that
    /// gives no orders.
    #[arg(long, conflicts_with = "floor")]
    idle: bool,
    /// How loosely it chooses. Nought takes what it likes best.
    #[arg(long, default_value_t = 0.0)]
    heat: f32,
    /// What the seat is there to do, one to five.
    #[arg(long, default_value_t = 2)]
    role: u8,
}

/// Writing out a fresh model.
#[derive(clap::Args, Debug)]
struct Fresh {
    /// Where to write it.
    #[arg(long, value_name = "FILE")]
    weights: Option<PathBuf>,
    /// What to draw the weights from.
    #[arg(long, default_value_t = 1)]
    seed: u64,
}

fn main() {
    let cli = Cli::parse();
    let doing = cli.doing.unwrap_or(Doing::Play(cli.playing));
    if let Err(err) = carry_out(doing) {
        eprintln!("bot: {err}");
        std::process::exit(1);
    }
}

/// Does what was asked for.
fn carry_out(doing: Doing) -> std::io::Result<()> {
    match doing {
        Doing::Shape => {
            say_the_shape();
            Ok(())
        }
        Doing::Fresh(asked) => {
            let path = asked.weights.unwrap_or_else(Model::path);
            let model = Model::fresh(asked.seed).map_err(std::io::Error::other)?;
            model.save(&path).map_err(std::io::Error::other)?;
            println!(
                "wrote {} weights to {}",
                model.weight_count(),
                path.display()
            );
            Ok(())
        }
        Doing::Play(asked) => join_a_match(asked),
        Doing::Train(asked) => train(asked),
        Doing::Descend(asked) => descend(asked),
        Doing::Judge(asked) => judge(asked),
        Doing::Duel(asked) => duel(asked),
    }
}

/// Teaches the model a lesson, and keeps only what measures better.
fn descend(asked: Descending) -> std::io::Result<()> {
    // Named or the whole ladder, as breeding does. A ladder held together by a
    // loop outside the program is a ladder nobody else can walk.
    let ladder: Vec<Lesson> = match asked.lesson {
        None => (1..=LESSONS as u8).filter_map(Lesson::of).collect(),
        Some(number) => match Lesson::of(number) {
            None => {
                return Err(std::io::Error::other(format!(
                    "lessons are numbered one to {LESSONS}"
                )));
            }
            Some(lesson) => vec![lesson],
        },
    };
    for lesson in ladder {
        descend_one(&asked, lesson)?;
    }
    Ok(())
}

/// Teaches one lesson by gradient, keeping only what measures better.
fn descend_one(asked: &Descending, lesson: Lesson) -> std::io::Result<()> {
    let Some(role) = Role::of(asked.role) else {
        return Err(std::io::Error::other("roles are numbered one to five"));
    };
    let weights = asked.weights.clone().unwrap_or_else(Model::path);
    let standing = Yard::default();
    let how = School {
        yard: Yard {
            server: asked.server.clone().unwrap_or(standing.server),
            builtin: !asked.on_the_wire,
            ..standing
        },
        lesson,
        role,
        rounds: asked.rounds,
        matches: asked.matches,
        lanes: asked.lanes,
        heat: asked.heat,
        rate: asked.rate,
        batch: asked.batch,
        most_frames: asked.frames,
        seed: asked.seed,
        weights: weights.clone(),
        ..School::for_lesson(lesson)
    };
    let model = if weights.exists() {
        Model::from_file(&weights, asked.seed).map_err(std::io::Error::other)?
    } else {
        println!("no weights at {}, starting fresh", weights.display());
        Model::fresh(asked.seed).map_err(std::io::Error::other)?
    };
    let mut adam = Adam::new(&model, how.rate).map_err(std::io::Error::other)?;
    let mut dice = Dice::from_seed(how.seed ^ 0x51ed_2701);
    // Which server, because it is found beside this binary rather than named,
    // and a stale one next to a fresh bot trains against a different game.
    println!(
        "teaching {} to a model of {} weights, matches of {} ticks, played by {}",
        lesson.name(),
        model.weight_count(),
        lesson.ticks(),
        how.yard.server.display()
    );

    // What it is worth before any of this, so that a run which never improves
    // leaves the file exactly as it found it.
    let working = weights.with_extension("learning");
    model.save(&working).map_err(std::io::Error::other)?;
    let mut best = measure(&how, &std::fs::read(&working)?)?;
    println!("starting out at {best:.1}");

    for round in 1..=how.rounds {
        model.save(&working).map_err(std::io::Error::other)?;
        let bytes = std::fs::read(&working)?;
        let rolls = gather(&how, &bytes, u64::from(round))?;
        let paid = if rolls.is_empty() {
            0.0
        } else {
            rolls.iter().map(|roll| roll.paid_in_all()).sum::<f32>() / rolls.len() as f32
        };
        let mut frames: Vec<bota_bot::Frame> =
            rolls.into_iter().flat_map(|roll| roll.frames).collect();
        let played = frames.len();
        thin(&mut frames, how.most_frames, &mut dice);
        let loss = learn_from(&model, &mut adam, &frames, &how, &mut dice);
        model.save(&working).map_err(std::io::Error::other)?;
        let mut kept = String::new();
        if round.is_multiple_of(how.measure_every) || round == how.rounds {
            let now = measure(&how, &std::fs::read(&working)?)?;
            kept = if now > best {
                best = now;
                model.save(&weights).map_err(std::io::Error::other)?;
                format!(", measured {now:.1}, kept")
            } else {
                format!(", measured {now:.1}, best still {best:.1}")
            };
        }
        // Both numbers, because the second is a cap and a cap that reported
        // only what it let through would read as everything there was.
        println!(
            "round {round}: learned from {} of {played}, matches paid {paid:.1}, loss {loss:.3}{kept}",
            frames.len()
        );
    }
    let _ = std::fs::remove_file(&working);
    println!("the best it managed was {best:.1}");
    Ok(())
}

/// Keeps a heap of frames down to the most the weights are moved by, taking
/// those it keeps at random.
fn thin(frames: &mut Vec<bota_bot::Frame>, most: usize, dice: &mut Dice) {
    if frames.len() <= most || most == 0 {
        return;
    }
    let mut order: Vec<usize> = (0..frames.len()).collect();
    for at in (1..order.len()).rev() {
        order.swap(at, (dice.next_u64() % (at as u64 + 1)) as usize);
    }
    order.truncate(most);
    order.sort_unstable();
    *frames = order.into_iter().map(|at| frames[at].clone()).collect();
}

/// Says what a model is shown and what it may choose.
fn say_the_shape() {
    println!(
        "shown: {NUMBERS} numbers a tick, {} frames of them, {} in all",
        bota_bot::HISTORY,
        bota_bot::INPUT
    );
    println!(
        "  frames from {} ticks back, in ticks: {:?}",
        bota_bot::REMEMBERED,
        bota_bot::AGES
    );
    for (name, size) in bota_bot::LAYOUT {
        println!("  {size:4}  {name}");
    }
    println!("deeds: {DEEDS}");
    for (name, size) in bota_bot::BLOCKS {
        println!("  {size:4}  {name}");
    }
}

/// Joins a server and plays one match.
fn join_a_match(asked: Playing) -> std::io::Result<()> {
    let Some(role) = Role::of(asked.role) else {
        return Err(std::io::Error::other(format!(
            "there is no role {}: they are numbered one to five",
            asked.role
        )));
    };
    let chair = Chair {
        addr: asked.addr,
        name: asked.name,
        hero: HeroId(asked.hero),
        limit: asked.limit,
        role,
        lesson: Lesson::GrowRich,
        until: None,
    };
    let mut idle = Nothing;
    let mut floor = FirstAllowed;
    let mut learned;
    let mind: &mut (dyn Mind + Send) = if asked.idle {
        &mut idle
    } else if asked.floor {
        &mut floor
    } else {
        let path = asked.weights.unwrap_or_else(Model::path);
        if !path.exists() {
            return Err(std::io::Error::other(format!(
                "no weights at {}: write some with `bota-bot fresh` first",
                path.display()
            )));
        }
        let model = Model::from_file(&path, 1).map_err(std::io::Error::other)?;
        learned = Learned::loosely(model, asked.heat, 1);
        &mut learned
    };
    let out = play(mind, &chair)?;
    let mine = out.mine.as_ref();
    println!(
        "played {} ticks: {} last hits, {} denies, {} kills, {} deaths",
        out.ticks,
        mine.map_or(0, |row| row.last_hits),
        mine.map_or(0, |row| row.denies),
        mine.map_or(0, |row| row.kills),
        mine.map_or(0, |row| row.deaths),
    );
    println!(
        "chose on {} ticks, {} of them something it had been told it could not do, \
         {} orders refused",
        out.chose, out.refused, out.rejected
    );
    for (reason, many) in &out.refusals {
        println!("  {many} orders refused: {reason:?}");
    }
    println!("what the lessons paid it:");
    for line in out.card.lines() {
        println!("  {line}");
    }
    Ok(())
}
