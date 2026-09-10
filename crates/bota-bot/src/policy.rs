//! The deterministic policy: one ladder of wants, walked top to bottom.
//!
//! Every tick the same questions are asked in the same order, and the first
//! one with an answer is what the seat does. Nothing is drawn at random and
//! nothing is remembered beyond the last snapshot, the attack cycle and how
//! long the courier has been idle, so the same match played twice goes the
//! same way.

use bota_proto::{
    DamageKind, EffectId, EventKind, HeroId, MatchInfo, MatchStats, SlotId, StatusFlags, Target,
    Team, UnitView, Vec2, WorldView,
};

use crate::{
    AGGRO_COOLDOWN, Ask, Beat, CLARITY, CLARITY_MANA, CREEP_ACQUISITION, Errands, FIGHT_RANGE,
    Field, Forest, HARASS_RANGE, LAST_HIT_SLACK, MAGIC_STICK, MAGIC_WAND, MENDED_RETURN,
    PULL_DRIFT, PULL_HEALTH, QUELLING_BLADE, QUELLING_BONUS, RESTORE_CHARGES, RESTORE_PER_CHARGE,
    RETREAT_HEALTH, RETURN_HEALTH, Role, SALVE, SALVE_HEALS, SALVE_HEALTH, SCROLL, SHADOW_FIEND,
    SHAKE_CREEPS, SHAKE_TICKS, STAND_BEHIND, SYLLA, Stall, Steady, TANGO, TANGO_HEALS,
    TANGO_HEALTH, TANGO_WALK, TOWER_ATTACK_RANGE, TOWER_KEEP_OUT, WORN_SLOTS, after_mitigation,
    fiend_aim, fiend_spell, next_point, order_by, point_along, span, sylla_spell,
};

/// The mending a salve or a tango puts on, as the wire names it.
const MENDING: EffectId = EffectId(1);

/// The mending a clarity puts on, as the wire names it.
const CLARITY_EFFECT: EffectId = EffectId(2);

/// How far a tango reaches for its tree.
const TANGO_REACH: f32 = 165.0;

/// How far from a tree a hero walking to one stops.
pub const TREE_STANDOFF: i32 = 110;

/// How near a hero has to stand to where it wants to be for it to be there.
const HOLD_SLACK: f32 = 220.0;

/// Ticks a swing already ordered is waited out before the hero moves on.
const SWING_PATIENCE: u32 = 45;

/// How far inside its own reach a hero stands from what it is swinging at.
const SWING_EDGE: f32 = 40.0;

/// How far behind the tower it is aimed at a scroll lands.
const PORTAL_STANDOFF: f32 = 300.0;

/// How far the lane has to be for a scroll to beat walking.
const PORTAL_WORTH: f32 = 3500.0;

/// Ticks a scroll leaves behind before another may be used.
const SCROLL_WAIT: u32 = 2100;

/// Health, as a part of the whole, below which a hero does not start
/// anything with a hero of the other side.
const HARASS_HEALTH: f32 = 0.6;

/// A bot that plays by a fixed set of rules.
pub struct Playbook {
    /// What the seat is there to do.
    role: Role,
    /// The seat it holds.
    slot: Option<SlotId>,
    /// The hero it plays.
    hero: HeroId,
    /// The shop, as the match described it.
    stall: Stall,
    /// The map's forest.
    forest: Forest,
    /// The attack cycle and the fall of everything in sight.
    beat: Beat,
    /// The want standing at the moment.
    steady: Steady,
    /// What the courier has been sent for.
    errands: Errands,
    /// The tick last seen.
    tick: u32,
    /// The body the seat drives, while one is standing.
    body: Option<bota_proto::EntityId>,
    /// Whether the hero is on its way out of the lane.
    pulling_out: bool,
    /// The swing last ordered, while it may still be under way.
    swinging_at: Option<Ask>,
    /// The tick that swing was ordered on.
    swinging_since: u32,
    /// The tick the last scroll was aimed. Absent while none has been.
    portalled_at: Option<u32>,
    /// The tick the creeps first set out.
    pregame: u32,
    /// The tick the creeps were last shaken off. Absent while none has been.
    shook_at: Option<u32>,
    /// The tick the creeps were last called on. Absent while none has been.
    pulled_at: Option<u32>,
    /// Which rung of the ladder answered last.
    rung: &'static str,
    /// Ticks between lines written about what the seat is doing. Nought
    /// writes none.
    trace: u32,
}

impl Playbook {
    /// A playbook for a seat playing a role.
    pub fn new(role: Role, hero: HeroId) -> Playbook {
        Playbook {
            role,
            slot: None,
            hero,
            stall: Stall::default(),
            forest: Forest::default(),
            beat: Beat::new(hero, 30),
            steady: Steady::new(),
            errands: Errands::new(),
            tick: 0,
            body: None,
            pulling_out: false,
            swinging_at: None,
            swinging_since: 0,
            portalled_at: None,
            pregame: 0,
            shook_at: None,
            pulled_at: None,
            rung: "none",
            trace: 0,
        }
    }

    /// Writes a line about what the seat is doing every so many ticks.
    pub fn trace_every(&mut self, ticks: u32) {
        self.trace = ticks;
    }

    /// One line about where the seat stands and what it decided.
    fn say(&self, field: &Field, ask: Option<Ask>) {
        let at = field.at();
        let creep = field
            .creeps
            .first()
            .map(|creep| field.gap_to(creep))
            .unwrap_or(f32::NAN);
        let held = |slots: &[Option<bota_proto::ItemView>]| {
            slots
                .iter()
                .flatten()
                .map(|had| match had.charges {
                    Some(left) => format!("{}x{left}", had.id.0),
                    None => format!("{}", had.id.0),
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        let bag = field.me.map_or(String::new(), |me| held(&me.items));
        let stash = field.seat.stash.as_ref().map_or(String::new(), |s| held(s));
        let riding = field.courier.map_or(String::from("-"), |b| held(&b.items));
        eprintln!(
            "{:>6}  at ({:.0},{:.0}) hp {:.2} mana {}/{} lvl {} gold {} \
             creeps {}/{} nearest {:.0} out {} bag[{bag}] stash[{stash}] cour[{riding}] \
             {} -> {:?}",
            self.tick,
            at.x.to_f32(),
            at.y.to_f32(),
            field.health(),
            field.me.map_or(0, |me| me.mana),
            field.me.map_or(0, |me| me.max_mana),
            field.seat.level,
            field.gold(),
            field.creeps.len(),
            field.own_creeps.len(),
            creep,
            self.pulling_out,
            self.rung,
            ask.map(|ask| ask.order),
        );
    }

    /// What the seat wants done this tick, before the want in hand is
    /// weighed against it.
    pub fn decide(&mut self, field: &Field) -> Option<Ask> {
        self.rung = "none";
        let me = field.me?;
        // Held, nothing the body is told will land; in the middle of a
        // channel, an order is how the channel is thrown away. Either way the
        // tick is not the seat's to spend.
        if me.statuses.bits & (StatusFlags::STUNNED | StatusFlags::CHANNELLING) != 0 {
            self.rung = "held";
            return None;
        }
        if let Some(slot) = next_point(field) {
            self.rung = "learn";
            return Some(Ask::learn(slot));
        }
        if let Some(errand) = self.errands.errand(self.tick, field) {
            self.rung = "errand";
            return Some(errand);
        }
        if let Some(buy) = self.shopping(field) {
            self.rung = "buy";
            return Some(buy);
        }
        if let Some(moved) = self.tidy(field) {
            self.rung = "tidy";
            return Some(moved);
        }
        // Settled before the drink, so that a mend is weighed against a walk
        // to the fountain that has already been decided on.
        self.mind_health(field);
        if let Some(remedy) = self.remedy(field) {
            self.rung = "remedy";
            return Some(remedy);
        }
        // Before the walk, since what the walk is for is the thing a scroll
        // does in three seconds.
        if let Some(back) = self.portal(field) {
            self.rung = "portal";
            self.portalled_at = Some(self.tick);
            return Some(back);
        }
        if let Some(away) = self.retreat(field) {
            self.rung = "retreat";
            return Some(away);
        }
        if let Some(spell) = self.spell(field) {
            self.rung = "spell";
            return Some(spell);
        }
        if let Some(spot) = self.aim(field) {
            self.rung = "aim";
            return Some(Ask::walk_to(spot));
        }
        if let Some(blow) = self.finish(field) {
            self.rung = "finish";
            self.swinging_at = field.me.map(|_| blow);
            self.swinging_since = self.tick;
            return Some(blow);
        }
        // A swing already under way is left alone. An order of any kind ends
        // the wind-up before it, so walking somewhere on the tick a blow was
        // about to land is that blow given up.
        if self.mid_swing(field) {
            self.rung = "mid-swing";
            return None;
        }
        if let Some(off) = self.shake(field) {
            self.rung = "shake";
            return Some(off);
        }
        if let Some(back) = self.pull(field) {
            self.rung = "pull";
            self.pulled_at = Some(self.tick);
            return Some(back);
        }
        if let Some(press) = self.press(field) {
            self.rung = "press";
            return Some(press);
        }
        self.rung = "hold";
        self.hold(field)
    }

    /// Whether a swing ordered a moment ago is still worth waiting out.
    fn mid_swing(&self, field: &Field) -> bool {
        let Some(Ask {
            order:
                bota_proto::Order::Attack {
                    target: Target::Unit(mark),
                },
            ..
        }) = self.swinging_at
        else {
            return false;
        };
        if self.tick.saturating_sub(self.swinging_since) > SWING_PATIENCE {
            return false;
        }
        field
            .view
            .units
            .iter()
            .find(|unit| unit.id == mark)
            .is_some_and(|unit| unit.hp > 0 && field.in_reach(unit))
    }

    /// The next thing on the shopping list, while there is quiet to buy it
    /// in.
    fn shopping(&self, field: &Field) -> Option<Ask> {
        if !field.at_shop() && field.foes_within(FIGHT_RANGE).next().is_some() {
            return None;
        }
        // Bought away from the shop, goods land in the stash, where only a
        // courier can reach them.
        if !field.at_shop() && field.courier.is_none() {
            return None;
        }
        self.stall.next_buy(field).map(Ask::buy)
    }

    /// A consumable worth spending now.
    ///
    /// Only while nobody of the other side is near: a mend a blow puts out is
    /// a mend thrown away.
    fn remedy(&self, field: &Field) -> Option<Ask> {
        let me = field.me?;
        let ready = |id| {
            field
                .item(id)
                .filter(|(_, held)| {
                    held.cooldown_left == 0
                        && held.mute_left == 0
                        && held.charges.is_none_or(|left| left > 0)
                })
                .map(|(slot, _)| slot)
        };
        // A wand is pressed in a fight as readily as out of one: what it
        // gives back arrives at once, and nothing puts it out.
        if let Some(press) = self.press_a_wand(field) {
            return Some(press);
        }
        if field.foes_within(FIGHT_RANGE).next().is_some() {
            return None;
        }
        let showing = |what: EffectId| me.effects.iter().any(|effect| effect.id == what);
        // A hero already walking to its fountain spends a mend only on one
        // that would turn the walk round. The fountain gives back
        // twenty-five a tick and asks nothing for it, so a mend that runs
        // out short of fighting health is a mend the fountain was about to
        // make free.
        let worth_mending = |heals: i32| {
            !self.pulling_out || me.hp + heals >= (MENDED_RETURN * me.max_hp as f32).round() as i32
        };
        if !showing(MENDING)
            && field.health() < SALVE_HEALTH
            && worth_mending(SALVE_HEALS)
            && let Some(slot) = ready(SALVE)
        {
            return Some(Ask::use_item(slot, Target::Unit(me.id)));
        }
        // A tango is paid for by a tree standing within reach of it, and a
        // lane is cleared of trees either side of its centre, so the walk to
        // one is part of eating it.
        if !showing(MENDING)
            && field.health() < TANGO_HEALTH
            && worth_mending(TANGO_HEALS)
            && let Some(slot) = ready(TANGO)
        {
            if let Some(tree) = self
                .forest
                .nearest_standing(field.view, me.pos, TANGO_REACH)
            {
                return Some(Ask::use_item(slot, Target::Pos(tree)));
            }
            if let Some(tree) = self.tree_to_eat(field) {
                return Some(Ask::walk_to(point_along(tree, me.pos, TREE_STANDOFF)));
            }
        }
        if !showing(CLARITY_EFFECT)
            && field.mana() < CLARITY_MANA
            && let Some(slot) = ready(CLARITY)
        {
            return Some(Ask::use_item(slot, Target::Unit(me.id)));
        }
        None
    }

    /// The nearest tree worth *walking* to for a tango.
    ///
    /// Only the ones no further up the lane than the hero already stands: a
    /// hero on a third of its health has no business walking towards the
    /// other side's fountain for a bite.
    fn tree_to_eat(&self, field: &Field) -> Option<Vec2> {
        let me = field.me?;
        let Some(lane) = field.lane.as_ref() else {
            return self.forest.nearest_standing(field.view, me.pos, TANGO_WALK);
        };
        let behind = lane.how_far_along(me.pos);
        self.forest
            .nearest_kept(field.view, me.pos, TANGO_WALK, |tree| {
                lane.how_far_along(tree) <= behind
            })
    }

    /// A wand or a stick worth pressing, for whichever pool it would fill.
    fn press_a_wand(&self, field: &Field) -> Option<Ask> {
        let me = field.me?;
        for id in [MAGIC_WAND, MAGIC_STICK] {
            let Some((slot, held)) = field.item(id) else {
                continue;
            };
            let charges = held.charges.unwrap_or(0);
            if held.cooldown_left > 0 || held.mute_left > 0 || charges < RESTORE_CHARGES {
                continue;
            }
            // Pressed only for what it would not waste: the pool it fills has
            // to be missing at least what the charges are worth.
            let worth = i32::from(charges) * RESTORE_PER_CHARGE;
            if me.max_hp - me.hp >= worth || me.max_mana - me.mana >= worth {
                return Some(Ask::use_item(slot, Target::None));
            }
        }
        None
    }

    /// Goods stranded in the backpack, moved to a working slot.
    ///
    /// A build lands in the lowest slot any of its parts came out of, and
    /// parts bought with the working slots full come out of the backpack, so
    /// a finished item can end up somewhere it does nothing.
    fn tidy(&self, field: &Field) -> Option<Ask> {
        let me = field.me?;
        let free = me
            .items
            .iter()
            .take(WORN_SLOTS)
            .position(|slot| slot.is_none())?;
        let stranded = me
            .items
            .iter()
            .enumerate()
            .skip(WORN_SLOTS)
            .find(|(_, slot)| slot.is_some())
            .map(|(at, _)| at)?;
        Some(Ask::mine(bota_proto::Order::Swap {
            from: bota_proto::ItemSlot(stranded as u8),
            to: bota_proto::ItemSlot(free as u8),
        }))
    }

    /// Settles whether the hero is on its way out of the lane.
    ///
    /// Kept apart from [`Playbook::retreat`] so that everything below it on
    /// the ladder — the scroll above all — reads a flag settled this tick
    /// rather than last.
    fn mind_health(&mut self, field: &Field) {
        let health = field.health();
        // A hero already being mended is on its way back rather than on its
        // way home: the walk to the fountain buys what the mend is already
        // paying for, and costs the lane the whole of the round trip.
        let mending = field
            .me
            .is_some_and(|me| me.effects.iter().any(|effect| effect.id == MENDING));
        let enough = if mending {
            MENDED_RETURN
        } else {
            RETURN_HEALTH
        };
        if health < RETREAT_HEALTH {
            self.pulling_out = true;
        } else if health >= enough {
            self.pulling_out = false;
        }
    }

    /// Where to go when the lane is no longer worth standing in.
    fn retreat(&mut self, field: &Field) -> Option<Ask> {
        if self.pulling_out {
            return field.home.map(Ask::walk_to);
        }
        // A tower with nothing of its own side's in front of it is not stood
        // under, whatever the health says.
        let me = field.me?;
        let lane = field.lane.as_ref()?;
        let tower = field
            .enemy_towers
            .iter()
            .find(|tower| span(tower.pos, me.pos) <= TOWER_KEEP_OUT as f32)?;
        let shielded = field
            .own_creeps
            .iter()
            .any(|creep| span(creep.pos, tower.pos) <= TOWER_ATTACK_RANGE as f32);
        if shielded {
            return None;
        }
        Some(Ask::walk_to(
            lane.spot_from(me.pos, -(TOWER_KEEP_OUT as f32)),
        ))
    }

    /// The scroll home, for a hero standing at its shop with the lane a walk
    /// away.
    ///
    /// A scroll lands where it is aimed, and what it is aimed at is the
    /// ground just behind the frontmost tower the lane still holds.
    ///
    /// The wait a scroll leaves behind is shared by every scroll a hero will
    /// ever hold, and it is not on the wire: the slot of a scroll bought to
    /// replace one just spent reads as ready. So the wait is counted here
    /// instead, from the tick the last one was aimed.
    fn portal(&self, field: &Field) -> Option<Ask> {
        // A scroll is not cast from anywhere in particular; what it asks is
        // that the spot it is aimed at stand by a building of one's own. So
        // the question is only whether where the hero is going is far enough
        // to be worth three seconds of standing still, and whether standing
        // still is safe. It carries both ways: out of the lane while hurt,
        // and back into it once mended.
        if self.tick < self.pregame {
            return None;
        }
        if field.foes_within(FIGHT_RANGE).next().is_some() {
            return None;
        }
        if self
            .portalled_at
            .is_some_and(|at| self.tick.saturating_sub(at) < SCROLL_WAIT)
        {
            return None;
        }
        let me = field.me?;
        let lane = field.lane.as_ref()?;
        let (slot, held) = field.item(SCROLL)?;
        if held.cooldown_left > 0 || held.mute_left > 0 || held.charges == Some(0) {
            return None;
        }
        let going = if self.pulling_out {
            lane.spot_from(field.home?, PORTAL_STANDOFF)
        } else {
            lane.spot_from(lane.mine?, -PORTAL_STANDOFF)
        };
        (span(going, me.pos) > PORTAL_WORTH).then(|| Ask::use_item(slot, Target::Pos(going)))
    }

    /// What the hero would cast this tick.
    fn spell(&self, field: &Field) -> Option<Ask> {
        match field.hero {
            SHADOW_FIEND => fiend_spell(field, &self.beat),
            SYLLA => sylla_spell(field, &self.beat),
            _ => None,
        }
    }

    /// Where to look so that a spell would land, for a hero whose spells are
    /// aimed by looking.
    fn aim(&self, field: &Field) -> Option<Vec2> {
        match field.hero {
            SHADOW_FIEND => fiend_aim(field),
            _ => None,
        }
    }

    /// A creep of either side worth swinging at right now.
    fn finish(&self, field: &Field) -> Option<Ask> {
        let me = field.me?;
        let worth = |mark: &UnitView, bonus: i32| {
            let gap = crate::gap_between(me, mark);
            if gap > me.attack_range.to_f32() {
                return false;
            }
            let damage = after_mitigation(me.attack_damage + bonus, DamageKind::Physical, mark);
            let left = self.beat.health_in(mark, self.beat.blow_lands_in(gap));
            left > 0 && left <= damage && left as f32 >= damage as f32 * LAST_HIT_SLACK
        };
        let bonus = if field.item(QUELLING_BLADE).is_some() {
            QUELLING_BONUS
        } else {
            0
        };
        // The worn one rather than the near one. A body only ever loses
        // health, so the lowest of them is the lowest again next tick, while
        // the nearest changes every time the wave shuffles — and a swing that
        // keeps changing its mind never lands.
        if let Some(mark) = field
            .creeps
            .iter()
            .copied()
            .filter(|creep| worth(creep, bonus))
            .min_by_key(|creep| creep.hp)
        {
            return Some(Ask::swing_at(mark.id));
        }
        let denied = field
            .own_creeps
            .iter()
            .copied()
            .filter(|creep| creep.hp * 2 <= creep.max_hp)
            .filter(|creep| worth(creep, bonus))
            .min_by_key(|creep| creep.hp)?;
        Some(Ask::swing_at(denied.id))
    }

    /// Shakes off the creeps that are chewing on the hero.
    ///
    /// An attack order aimed at one of your own makes every enemy creep near
    /// enough to have seen it pick a target afresh with the one who gave it
    /// put last. The order alone does it, whether the attack ever happens or
    /// not, and unlike calling creeps *on*, letting them go waits on nothing
    /// and spends nothing.
    ///
    /// It has to be aimed at somebody *else* of its own: a hero pointing at
    /// itself has told the creeps nothing. The nearest of its own creeps is
    /// what it points at, and the tick after, it is told to stand, since an
    /// order at a unit is a follow and the follow would walk it out of the
    /// spot it was holding.
    fn shake(&mut self, field: &Field) -> Option<Ask> {
        // An order at a unit is a follow, so the tick after the shake the
        // hero is walking to whatever it pointed at. Standing still puts that
        // right; the creeps were let go the moment the order was taken and do
        // not come back for it.
        //
        // The tick this answers on is not itself a shake, so it does not
        // start the clock again: doing that would leave every tick the tick
        // after a shake, and the hero standing still for good.
        if self.shook_at == Some(self.tick.saturating_sub(1)) {
            return Some(Ask::mine(bota_proto::Order::Move {
                target: Target::None,
            }));
        }
        if self
            .shook_at
            .is_some_and(|at| self.tick.saturating_sub(at) < SHAKE_TICKS)
        {
            return None;
        }
        let biting = field
            .creeps
            .iter()
            .filter(|creep| self.beat.struck_me(creep.id))
            .count();
        if biting < SHAKE_CREEPS {
            return None;
        }
        // Somebody else of its own, and the nearest of them, so the follow it
        // turns into is the shortest one going.
        let ally = field
            .own_creeps
            .first()
            .map(|creep| creep.id)
            .or_else(|| field.allies.first().map(|hero| hero.id))
            .or_else(|| field.courier.map(|bird| bird.id))?;
        self.shook_at = Some(self.tick);
        Some(Ask::swing_at(ally))
    }

    /// Calls the other side's creeps onto the hero, to drag where the waves
    /// meet back down the lane.
    ///
    /// An attack order aimed at an enemy hero makes every enemy creep within
    /// its own acquisition of the one who gave it come for him instead, for
    /// [`AGGRO_HOLD`] ticks. Standing behind its own line, the hero is a spot
    /// they have to walk past that line to reach, and the fight comes with
    /// them.
    ///
    /// What it costs is the blows they land on the way, which is why it waits
    /// on health and on the wave having actually been pushed.
    fn pull(&self, field: &Field) -> Option<Ask> {
        let lane = field.lane.as_ref()?;
        if field.health() < PULL_HEALTH {
            return None;
        }
        if self
            .pulled_at
            .is_some_and(|at| self.tick.saturating_sub(at) < AGGRO_COOLDOWN)
        {
            return None;
        }
        // Only what is near enough to hear the order is worth giving it for.
        let near = field
            .creeps
            .first()
            .filter(|creep| field.gap_to(creep) <= CREEP_ACQUISITION as f32)?;
        let meet = lane.how_far_along(lane.where_they_meet());
        if lane.how_far_along(near.pos) - meet < PULL_DRIFT {
            return None;
        }
        // The order names a hero; which creeps answer is settled by where
        // they stand, so how far off that hero is does not matter.
        let mark = field.enemies.first()?;
        Some(Ask::swing_at(mark.id))
    }

    /// An enemy or a building worth walking at.
    fn press(&self, field: &Field) -> Option<Ask> {
        // Somebody of the other side standing close enough to swing at,
        // while there is health to spare for what swinging at them calls
        // down.
        if field.health() >= HARASS_HEALTH
            && let Some(mark) = field
                .foes_within(HARASS_RANGE)
                .find(|foe| field.in_reach(foe))
        {
            return Some(Ask::swing_at(mark.id));
        }
        // With the lane clear, whatever of theirs stands next in it.
        if !field.creeps.is_empty() {
            return None;
        }
        // Whatever of theirs stands next in the lane, and only with its own
        // creeps in front of it: a hero alone under a tower is a hero the
        // tower shoots.
        let works = field.enemy_works.first()?;
        let shielded = field
            .own_creeps
            .iter()
            .any(|creep| span(creep.pos, works.pos) <= TOWER_ATTACK_RANGE as f32);
        if !shielded {
            return None;
        }
        if field.in_reach(works) {
            return Some(Ask::swing_at(works.id));
        }
        Some(Ask::fight_towards(works.pos))
    }

    /// Where to stand while there is nothing else to do.
    ///
    /// Behind its own creep line while the waves are in contact, and at the
    /// spot they meet while they are not. A hero already standing within
    /// [`HOLD_SLACK`] of that spot is told nothing at all: an order would end
    /// whatever swing it was in the middle of, for a step it did not need.
    fn hold(&self, field: &Field) -> Option<Ask> {
        let me = field.me?;
        let lane = field.lane.as_ref()?;
        let spot = match field.creeps.first() {
            // Just inside a swing of the nearest of theirs, and no nearer:
            // standing among them is four creeps' worth of blows for nothing
            // a swing from further back would not have reached.
            Some(creep) => lane.spot_from(creep.pos, -(me.attack_range.to_f32() - SWING_EDGE)),
            None => {
                let front = field
                    .own_creeps
                    .iter()
                    .map(|creep| creep.pos)
                    .max_by(|one, other| {
                        order_by(lane.how_far_along(*one), lane.how_far_along(*other))
                    });
                match front {
                    Some(front) => {
                        let behind = lane.spot_from(front, -(STAND_BEHIND as f32));
                        let meet = lane.where_they_meet();
                        // Never back towards its own end to meet a wave that
                        // has not set out yet.
                        if lane.how_far_along(behind) > lane.how_far_along(meet) {
                            behind
                        } else {
                            meet
                        }
                    }
                    None => lane.where_they_meet(),
                }
            }
        };
        (span(spot, me.pos) > HOLD_SLACK).then(|| Ask::walk_to(spot))
    }
}

impl crate::Bot for Playbook {
    fn seated(&mut self, slot: Option<SlotId>) {
        self.slot = slot;
    }

    fn match_started(&mut self, info: &MatchInfo) {
        if let Some(pick) = info.picks.iter().find(|pick| Some(pick.slot) == self.slot) {
            self.hero = pick.hero;
        }
        self.stall = Stall::of(info);
        self.forest = Forest::of(info);
        self.pregame = info.pregame_ticks;
        self.beat = Beat::new(self.hero, info.tick_rate);
    }

    fn on_tick(&mut self, view: &WorldView) -> Option<Ask> {
        let slot = self.slot?;
        self.tick = view.tick;
        let field = Field::of(view, slot, self.role)?;
        self.body = field.me.map(|me| me.id);
        self.beat.watch(view, field.me);
        self.errands.watch(view.tick, &field);
        let ask = self.decide(&field);
        if self.trace > 0 && view.tick.is_multiple_of(self.trace) {
            self.say(&field, ask);
        }
        ask.filter(|ask| self.steady.worth_sending(view.tick, *ask))
    }

    fn on_events(&mut self, tick: u32, events: &[EventKind]) {
        self.beat.saw(tick, events, self.body);
    }

    fn finished(&mut self, _winner: Team, _stats: &MatchStats) {
        self.steady.forget();
    }
}
