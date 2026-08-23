//! A training run as it is written down.
//!
//! A plan is a file: a sequence of stages, each naming a lesson and how long a
//! crowd is bred at it. Reading one gives a [`Term`] per stage — the lesson
//! with its clock, and the crowd that is bred at it — which is everything
//! [`teach_a_lesson`](crate::teach_a_lesson) needs.
//!
//! A stage may leave anything but `score` out. What it does not say comes from
//! the plan's `defaults`, and what those do not say either comes from the
//! lesson's rung of the ladder and from a plain crowd.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{LADDER, Lesson, Role, Rung, Tribe, Yard};

/// A whole training run, as a file spells it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// The server to run. Absent uses the one built beside this binary.
    #[serde(default)]
    pub server: Option<PathBuf>,
    /// Whether matches are played over a socket instead of in this process.
    #[serde(default)]
    pub on_the_wire: bool,
    /// What a stage falls back to for anything it does not say.
    #[serde(default)]
    pub defaults: Stage,
    /// The stages, taught in the order they are written.
    pub sequence: Vec<Stage>,
}

/// One stage of a plan, with everything it does not say left out.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    /// Which lesson is paid for, spelled with underscores. Absent only in the
    /// defaults.
    #[serde(default)]
    pub score: Option<String>,
    /// Ticks a match runs, which is also how long the lesson is paid for.
    #[serde(default)]
    pub ticks: Option<u32>,
    /// Generations the stage runs for.
    #[serde(default)]
    pub generations: Option<u32>,
    /// How many models there are.
    #[serde(default)]
    pub population: Option<usize>,
    /// Matches each model plays a generation.
    #[serde(default)]
    pub matches: Option<usize>,
    /// How many of the crowd survive a generation.
    #[serde(default)]
    pub survivors: Option<usize>,
    /// How far a child is moved from its parent.
    #[serde(default)]
    pub mutation: Option<f32>,
    /// How many matches run at once.
    #[serde(default)]
    pub lanes: Option<usize>,
    /// What the seats are there to do.
    #[serde(default)]
    pub role: Option<String>,
    /// Where the stage is seeded from.
    #[serde(default)]
    pub seed: Option<u64>,
}

/// One stage with everything settled.
#[derive(Clone, Debug)]
pub struct Term {
    /// The lesson and the clock its matches run to.
    pub rung: Rung,
    /// The crowd, and how it is bred.
    pub tribe: Tribe,
}

impl Plan {
    /// The plan a file spells.
    pub fn read(path: &Path) -> Result<Plan, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|wrong| format!("{}: {wrong}", path.display()))?;
        Plan::of(&text).map_err(|wrong| format!("{}: {wrong}", path.display()))
    }

    /// The plan a piece of text spells.
    pub fn of(text: &str) -> Result<Plan, String> {
        serde_yaml::from_str(text).map_err(|wrong| wrong.to_string())
    }

    /// Every stage settled, in the order they are taught.
    ///
    /// Everything a plan can get wrong is answered here rather than a stage at
    /// a time: a run of several hours that stops on its sixth stage because a
    /// number there is nonsense has thrown away the five before it.
    pub fn terms(&self) -> Result<Vec<Term>, String> {
        if self.sequence.is_empty() {
            return Err("a plan with an empty sequence teaches nothing".to_string());
        }
        self.sequence
            .iter()
            .enumerate()
            .map(|(at, stage)| {
                self.term_of(stage)
                    .map_err(|wrong| format!("stage {}: {wrong}", at + 1))
            })
            .collect()
    }

    /// One stage settled against the defaults.
    fn term_of(&self, stage: &Stage) -> Result<Term, String> {
        let fall_back = &self.defaults;
        let named = stage
            .score
            .as_ref()
            .or(fall_back.score.as_ref())
            .ok_or_else(|| format!("no score. It is one of: {}", spellings()))?;
        let Some(lesson) = Lesson::named(named) else {
            return Err(format!(
                "no lesson is called {named}. It is one of: {}",
                spellings()
            ));
        };
        let ticks = stage.ticks.or(fall_back.ticks).unwrap_or(lesson.ticks());
        if ticks == 0 {
            return Err("a match of nought ticks is scored on nothing".to_string());
        }

        let plain = Tribe::new(POPULATION, MATCHES);
        let population = stage
            .population
            .or(fall_back.population)
            .unwrap_or(plain.folk);
        // A crowd of one is a crowd that cannot be sorted, and being handed two
        // when two were not asked for is worse than being told.
        if population < 2 {
            return Err(format!(
                "a population of {population} has nothing to be chosen over; it is two or more"
            ));
        }
        let matches = stage.matches.or(fall_back.matches).unwrap_or(plain.trials);
        if matches == 0 {
            return Err("a model that plays no matches is judged on nothing".to_string());
        }
        let generations = stage
            .generations
            .or(fall_back.generations)
            .unwrap_or(plain.lives);
        if generations == 0 {
            return Err("a stage of nought generations breeds nothing".to_string());
        }
        let survivors = stage
            .survivors
            .or(fall_back.survivors)
            .unwrap_or((population / 4).max(1));
        if survivors == 0 || survivors > population {
            return Err(format!(
                "{survivors} survivors of a population of {population}; it is one to {population}"
            ));
        }
        let mutation = stage
            .mutation
            .or(fall_back.mutation)
            .unwrap_or(plain.spread);
        if mutation.is_nan() || mutation < 0.0 {
            return Err(format!("a mutation of {mutation} moves a child nowhere"));
        }
        let lanes = stage.lanes.or(fall_back.lanes).unwrap_or(plain.lanes);
        if lanes == 0 {
            return Err("nought lanes runs no matches at all".to_string());
        }
        let role = match stage.role.as_ref().or(fall_back.role.as_ref()) {
            None => plain.role,
            Some(named) => Role::named(named)
                .ok_or_else(|| format!("no role is called {named}. It is one of: {}", roles()))?,
        };
        let seed = stage.seed.or(fall_back.seed).unwrap_or(plain.seed);

        let standing = Yard::default();
        Ok(Term {
            rung: Rung {
                ticks,
                ..*lesson.rung()
            },
            tribe: Tribe {
                yard: Yard {
                    server: self.server.clone().unwrap_or(standing.server),
                    builtin: !self.on_the_wire,
                    ..standing
                },
                role,
                folk: population,
                trials: matches,
                lives: generations,
                keep: survivors,
                spread: mutation,
                lanes,
                seed,
            },
        })
    }
}

/// The population a stage that names none is bred at.
const POPULATION: usize = 10;
/// The matches a model plays a generation when a stage names none.
const MATCHES: usize = 1;

/// How the lessons are spelled in a plan, side by side.
fn spellings() -> String {
    LADDER
        .iter()
        .map(|rung| rung.lesson.spelling())
        .collect::<Vec<String>>()
        .join(", ")
}

/// How the roles are spelled in a plan, side by side.
fn roles() -> String {
    (1..=5)
        .filter_map(Role::of)
        .map(Role::spelling)
        .collect::<Vec<&str>>()
        .join(", ")
}
