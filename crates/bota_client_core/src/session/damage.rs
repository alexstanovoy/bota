//! Last observed damage ticks, not an amount history. No server state is read.
use bota_proto::SlotId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageSource {
    Hero(Option<SlotId>),
    Creep,
    Tower,
    Other,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecentDamage {
    hero: Option<u32>,
    creep: Option<u32>,
    tower: Option<u32>,
    by_hero: [Option<u32>; bota_proto::limits::MAX_SEATS],
}
impl RecentDamage {
    /// O(1), no allocation; called only for positive damage reported by the seat's stream.
    pub(crate) fn record(&mut self, tick: u32, source: DamageSource) {
        match source {
            DamageSource::Hero(slot) => {
                self.hero = Some(tick);
                if let Some(slot) = slot.and_then(|slot| self.by_hero.get_mut(usize::from(slot.0)))
                {
                    *slot = Some(tick);
                }
            }
            DamageSource::Creep => self.creep = Some(tick),
            DamageSource::Tower => self.tower = Some(tick),
            DamageSource::Other => {}
        }
    }
    /// O(1). A missing timestamp means no such damage was observed while this entity was retained.
    pub fn last(&self, source: DamageSource) -> Option<u32> {
        match source {
            DamageSource::Hero(Some(slot)) => {
                self.by_hero.get(usize::from(slot.0)).copied().flatten()
            }
            DamageSource::Hero(None) => self.hero,
            DamageSource::Creep => self.creep,
            DamageSource::Tower => self.tower,
            DamageSource::Other => None,
        }
    }
    /// O(1). Strict interval boundary.
    pub fn within(&self, now: u32, window: u32, source: DamageSource) -> bool {
        self.last(source)
            .and_then(|tick| now.checked_sub(tick))
            .is_some_and(|age| age < window)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_timestamps_survive_other_damage_and_use_a_strict_window() {
        let mut history = RecentDamage::default();
        history.record(10, DamageSource::Hero(Some(SlotId(0))));
        history.record(11, DamageSource::Creep);
        history.record(12, DamageSource::Tower);
        assert_eq!(history.last(DamageSource::Hero(None)), Some(10));
        assert!(history.within(39, 30, DamageSource::Hero(Some(SlotId(0)))));
        assert!(!history.within(40, 30, DamageSource::Hero(Some(SlotId(0)))));
        assert!(!history.within(9, 30, DamageSource::Hero(None)));
        assert!(!history.within(12, 30, DamageSource::Hero(Some(SlotId(1)))));
        assert!(!history.within(12, 30, DamageSource::Hero(Some(SlotId(255)))));
    }
}
