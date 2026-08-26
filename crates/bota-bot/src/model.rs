//! The model, and the only place that knows what it is built with.
//!
//! One head over one trunk: a number per deed.
//!
//! The trunk is fed several ticks at once rather than only the newest, because
//! a swing that has begun, a creep that is about to die and a creep that has
//! just died look the same in one frame. The frames are spaced by doubling
//! ages — half a second back, then one, two, four, eight and sixteen — so the
//! window reaches a quarter of a minute on seven frames. Frames rather than a
//! memory of its own: a memory that has to be carried through twelve thousand
//! ticks and reset on every death costs more to train than that is worth.
//!
//! **The mask is applied to the numbers, not to the choice afterwards.** A
//! deed that cannot be done has its number sent to nothing before anything is
//! compared, so it can never come out on top and never has a gradient. Picking
//! freely and taking the points off later was considered and dropped: there is
//! one order a tick, so a wasted pick is a lost creep.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Result as Chance, Tensor, Var};

use crate::{DEEDS, Mind, NUMBERS, Shown};

/// Ticks in a second, which is what the ages are reckoned in.
pub const A_SECOND: u32 = 30;

/// How many frames the model is shown at once.
pub const HISTORY: usize = 7;

/// How long before the tick being decided each frame was seen, in ticks,
/// oldest first. The last is nought: the tick itself.
pub const AGES: [u32; HISTORY] = [
    16 * A_SECOND,
    8 * A_SECOND,
    4 * A_SECOND,
    2 * A_SECOND,
    A_SECOND,
    A_SECOND / 2,
    0,
];

/// How far back the oldest frame reaches, in ticks.
pub const REMEMBERED: u32 = AGES[0];

/// How wide the trunk is.
pub const WIDTH: usize = 256;
/// How many numbers go in altogether.
pub const INPUT: usize = NUMBERS * HISTORY;

/// The weights, and the machinery to run them.
pub struct Model {
    /// The trunk, then the head.
    trunk: Vec<Layer>,
    policy: Layer,
    /// Where the numbers live.
    device: Device,
}

/// One layer: what it multiplies by and what it adds.
struct Layer {
    weight: Var,
    bias: Var,
}

impl Layer {
    /// A layer of the given shape, drawn small and random.
    fn fresh(into: usize, out: usize, dice: &mut Dice, device: &Device) -> Chance<Layer> {
        let spread = (2.0f32 / into as f32).sqrt();
        let weights: Vec<f32> = (0..into * out).map(|_| dice.spread() * spread).collect();
        Ok(Layer {
            weight: Var::from_tensor(&Tensor::from_vec(weights, (into, out), device)?)?,
            bias: Var::from_tensor(&Tensor::zeros(out, DType::F32, device)?)?,
        })
    }

    /// Runs it.
    fn run(&self, rows: &Tensor) -> Chance<Tensor> {
        rows.matmul(self.weight.as_tensor())?
            .broadcast_add(self.bias.as_tensor())
    }
}

impl Model {
    /// Where trained weights are kept: beside the repository, not inside it.
    pub fn path() -> PathBuf {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        here.parent()
            .and_then(Path::parent)
            .unwrap_or(here)
            .join("weights.safetensors")
    }

    /// A model with weights drawn small and random, from a seed of our own.
    pub fn fresh(seed: u64) -> Chance<Model> {
        let device = Device::Cpu;
        let mut dice = Dice::from_seed(seed);
        Ok(Model {
            trunk: vec![
                Layer::fresh(INPUT, WIDTH, &mut dice, &device)?,
                Layer::fresh(WIDTH, WIDTH, &mut dice, &device)?,
            ],
            policy: Layer::fresh(WIDTH, DEEDS, &mut dice, &device)?,
            device,
        })
    }

    /// How many numbers the whole of it is.
    pub fn weight_count(&self) -> usize {
        self.weights()
            .iter()
            .map(|weight| weight.elem_count())
            .sum()
    }

    /// A number per deed, for every row given.
    pub fn run(&self, rows: &Tensor) -> Chance<Tensor> {
        let mut out = rows.clone();
        for layer in &self.trunk {
            out = layer.run(&out)?.tanh()?;
        }
        self.policy.run(&out)
    }

    /// The same for one tick's worth of numbers.
    pub fn weigh(&self, numbers: &[f32]) -> Chance<Vec<f32>> {
        let rows = Tensor::from_vec(numbers.to_vec(), (1, INPUT), &self.device)?;
        self.run(&rows)?.flatten_all()?.to_vec1()
    }

    /// Every number the model is made of, layer by layer.
    pub fn weights(&self) -> Vec<&Var> {
        let mut out: Vec<&Var> = Vec::new();
        for layer in self.trunk.iter().chain([&self.policy]) {
            out.push(&layer.weight);
            out.push(&layer.bias);
        }
        out
    }

    /// Every number the model is made of, laid end to end.
    ///
    /// The order is [`weights`](Model::weights), which is fixed, so two models
    /// of the same shape pour out numbers that line up one for one.
    pub fn pour(&self) -> Chance<Vec<f32>> {
        let mut out = Vec::with_capacity(self.weight_count());
        for weight in self.weights() {
            out.extend(weight.flatten_all()?.to_vec1::<f32>()?);
        }
        Ok(out)
    }

    /// Puts numbers back, in the order [`pour`](Model::pour) took them out.
    pub fn soak(&self, numbers: &[f32]) -> Chance<()> {
        let device = Device::Cpu;
        let mut at = 0;
        for weight in self.weights() {
            let shape = weight.shape().clone();
            let many = shape.elem_count();
            let Some(slice) = numbers.get(at..at + many) else {
                return Err(candle_core::Error::Msg(format!(
                    "wanted {many} numbers at {at}, was given {}",
                    numbers.len()
                )));
            };
            weight.set(&Tensor::from_vec(slice.to_vec(), shape, &device)?)?;
            at += many;
        }
        Ok(())
    }

    /// Writes the weights out.
    pub fn save(&self, path: &Path) -> Chance<()> {
        let mut named: HashMap<String, Tensor> = HashMap::new();
        for (at, weight) in self.weights().iter().enumerate() {
            named.insert(format!("w{at}"), weight.as_detached_tensor());
        }
        candle_core::safetensors::save(&named, path)
    }

    /// A model with the weights a block of bytes holds.
    pub fn from_bytes(bytes: &[u8], seed: u64) -> Chance<Model> {
        let model = Model::fresh(seed)?;
        let held = candle_core::safetensors::load_buffer(bytes, &Device::Cpu)?;
        for (at, weight) in model.weights().iter().enumerate() {
            if let Some(kept) = held.get(&format!("w{at}")) {
                weight.set(kept)?;
            }
        }
        Ok(model)
    }

    /// A model with the weights a file holds.
    pub fn from_file(path: &Path, seed: u64) -> Chance<Model> {
        let model = Model::fresh(seed)?;
        let held = candle_core::safetensors::load(path, &Device::Cpu)?;
        for (at, weight) in model.weights().iter().enumerate() {
            if let Some(kept) = held.get(&format!("w{at}")) {
                weight.set(kept)?;
            }
        }
        Ok(model)
    }
}

/// The model, playing.
///
/// Holds the last few ticks, because that is what it is shown, and nothing
/// else: everything about the game reached it through [`Shown`].
pub struct Learned {
    /// What was learned.
    pub model: Model,
    /// How loosely it chooses. Nought always takes what it likes best.
    pub heat: f32,
    /// Every tick it has seen inside the window, oldest first, each with the
    /// tick it was seen on.
    seen: VecDeque<(u32, Vec<f32>)>,
    /// Where a loose choice is drawn from.
    dice: Dice,
}

impl Learned {
    /// A bot playing by these weights, taking what it likes best.
    pub fn new(model: Model) -> Learned {
        Learned {
            model,
            heat: 0.0,
            seen: VecDeque::new(),
            dice: Dice::from_seed(1),
        }
    }

    /// The same, choosing loosely.
    pub fn loosely(model: Model, heat: f32, seed: u64) -> Learned {
        Learned {
            heat,
            dice: Dice::from_seed(seed),
            ..Learned::new(model)
        }
    }

    /// The frames [`AGES`] asks for, laid end to end, oldest first.
    ///
    /// Each is the newest tick seen at or before that many ticks ago, so a
    /// gap — a stretch where the seat chose nothing because there was nothing
    /// to choose — shows as the last thing it did see rather than moving the
    /// other frames along. Ages are counted in the match's own ticks, not in
    /// how many times this has been called.
    ///
    /// Before the window has filled, the oldest tick it has stands in for the
    /// ones that have not happened: a match should not begin by being shown
    /// noughts it will never see again.
    pub(crate) fn history(&mut self, at: u32, numbers: &[f32]) -> Vec<f32> {
        self.seen.push_back((at, numbers.to_vec()));
        // One frame is kept from beyond the window, because the oldest age
        // asks for the newest tick at or before it and that may be older than
        // the window itself when the seat has not chosen for a while.
        let oldest = at.saturating_sub(REMEMBERED);
        while self.seen.len() > 1 && self.seen[1].0 <= oldest {
            self.seen.pop_front();
        }
        let mut out = Vec::with_capacity(INPUT);
        for age in AGES {
            let wanted = at.saturating_sub(age);
            let after = self.seen.partition_point(|(was, _)| *was <= wanted);
            out.extend_from_slice(&self.seen[after.saturating_sub(1)].1);
        }
        out
    }
}

impl Mind for Learned {
    fn choose(&mut self, shown: &Shown) -> Option<usize> {
        if !shown.anything_to_do() {
            return None;
        }
        let numbers = self.history(shown.at, &shown.numbers);
        let liking = self.model.weigh(&numbers).ok()?;
        Some(pick(&liking, &shown.allowed, self.heat, &mut self.dice))
    }

    fn starting(&mut self) {
        self.seen.clear();
    }
}

/// Which deed to take, of the ones allowed.
///
/// What is not allowed is not compared at all. Drawing loosely is drawing in
/// proportion to how much the model likes each, which is how a policy finds
/// out whether what it believes is true.
pub fn pick(liking: &[f32], allowed: &[bool], heat: f32, dice: &mut Dice) -> usize {
    let legal = |at: usize| allowed.get(at).copied().unwrap_or(false);
    let best = |()| {
        (0..liking.len())
            .filter(|at| legal(*at))
            .fold((0usize, f32::MIN), |(best, most), at| {
                if liking[at] > most {
                    (at, liking[at])
                } else {
                    (best, most)
                }
            })
            .0
    };
    if heat <= 0.0 {
        return best(());
    }
    let highest = (0..liking.len())
        .filter(|at| legal(*at))
        .fold(f32::MIN, |most, at| most.max(liking[at]));
    let weights: Vec<f32> = (0..liking.len())
        .map(|at| {
            if legal(at) {
                ((liking[at] - highest) / heat).exp()
            } else {
                0.0
            }
        })
        .collect();
    let total: f32 = weights.iter().sum();
    if !total.is_finite() || total <= 0.0 {
        return best(());
    }
    let mut drawn = dice.unit() * total;
    for (at, weight) in weights.iter().enumerate() {
        drawn -= weight;
        if drawn <= 0.0 && *weight > 0.0 {
            return at;
        }
    }
    best(())
}

/// A small deterministic source of numbers, for drawing weights and choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dice {
    state: u64,
}

impl Dice {
    /// A stream from a seed.
    pub fn from_seed(seed: u64) -> Dice {
        Dice {
            state: seed ^ 0x9e37_79b9_7f4a_7c15,
        }
    }

    /// The next number of the stream.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number from nought up to one.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// A number about nought, most of them within one of it.
    pub fn spread(&mut self) -> f32 {
        let one = self.unit().max(f32::MIN_POSITIVE);
        let other = self.unit();
        (-2.0 * one.ln()).sqrt() * (std::f32::consts::TAU * other).cos()
    }
}
