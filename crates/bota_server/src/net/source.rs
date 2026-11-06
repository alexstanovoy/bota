//! Who a connection comes from, as the bounds on connections that have not said Hello count it.
//!
//! An address counts in the groups it belongs to, each with a bound of its own: an IPv4 address
//! (also one mapped into IPv6) alone; an IPv6 address in its /64, since one subscriber is handed a
//! whole /64 and could take a fresh address for every connection, and in its /48, which one site
//! or one host of a provider holds. Behind a reverse proxy every peer is the proxy's address: the
//! server reads no `X-Forwarded-For`, which any peer could forge.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex, MutexGuard};

/// Connections not past Hello one IPv4 address may hold.
pub const MAX_PER_IPV4: usize = 16;
/// Connections not past Hello one IPv6 /64 may hold.
pub const MAX_PER_IPV6_64: usize = 16;
/// Connections not past Hello one IPv6 /48 may hold.
pub const MAX_PER_IPV6_48: usize = 64;

/// A group of addresses a bound counts: its prefix, and how many bits long the prefix is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Group {
    prefix: IpAddr,
    bits: u8,
}

/// The narrowest group of `ip`: its source, the one connection is told apart by.
pub fn source(ip: IpAddr) -> Group {
    groups(ip)[0].0
}

/// The groups `ip` counts in, narrowest first, each with its bound.
pub fn groups(ip: IpAddr) -> Vec<(Group, usize)> {
    let v6 = match ip {
        IpAddr::V4(v4) => return vec![(prefixed(IpAddr::V4(v4), 32), MAX_PER_IPV4)],
        IpAddr::V6(v6) => v6,
    };
    if let Some(v4) = v6.to_ipv4_mapped() {
        return vec![(prefixed(IpAddr::V4(v4), 32), MAX_PER_IPV4)];
    }
    let cut = |bits: u8| {
        let mask = u128::MAX << (128 - u32::from(bits));
        prefixed(IpAddr::V6(Ipv6Addr::from_bits(v6.to_bits() & mask)), bits)
    };
    vec![(cut(64), MAX_PER_IPV6_64), (cut(48), MAX_PER_IPV6_48)]
}

fn prefixed(prefix: IpAddr, bits: u8) -> Group {
    Group { prefix, bits }
}

/// How many places each group holds of something bounded per source.
#[derive(Clone, Default)]
pub struct Places(Arc<Mutex<HashMap<Group, usize>>>);

impl Places {
    fn lock(&self) -> MutexGuard<'_, HashMap<Group, usize>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A place in every group of `ip`, held until the [`Place`] is dropped; none when any of
    /// them holds its bound already.
    pub fn take(&self, ip: IpAddr) -> Option<Place> {
        let groups = groups(ip);
        let mut held = self.lock();
        if groups
            .iter()
            .any(|(group, most)| held.get(group).is_some_and(|count| count >= most))
        {
            return None;
        }
        for (group, _) in &groups {
            *held.entry(*group).or_insert(0) += 1;
        }
        Some(Place {
            places: self.clone(),
            groups: groups.into_iter().map(|(group, _)| group).collect(),
        })
    }
}

/// One connection's places, given back on drop.
pub struct Place {
    places: Places,
    groups: Vec<Group>,
}

impl Drop for Place {
    fn drop(&mut self) {
        let mut held = self.places.lock();
        for group in &self.groups {
            if let Some(count) = held.get_mut(group) {
                *count -= 1;
                if *count == 0 {
                    held.remove(group);
                }
            }
        }
    }
}

/// Keyless watchers one group of addresses may hold, in every group alike: they are never let
/// go for room, so a few per source are all one source may have.
pub const MAX_WATCHERS_PER_GROUP: usize = 4;

/// Whether a connection from `ip` would pass a bound of its groups, counting `peers`, the
/// addresses of the connections already held.
pub fn over_bound(ip: IpAddr, peers: impl Iterator<Item = IpAddr> + Clone) -> bool {
    over(ip, peers, |most| most)
}

/// Whether a watcher from `ip` would pass [`MAX_WATCHERS_PER_GROUP`] in a group of its,
/// counting `peers`, the addresses of the keyless watchers already held.
pub fn over_watchers_bound(ip: IpAddr, peers: impl Iterator<Item = IpAddr> + Clone) -> bool {
    over(ip, peers, |_| MAX_WATCHERS_PER_GROUP)
}

fn over(
    ip: IpAddr,
    peers: impl Iterator<Item = IpAddr> + Clone,
    bound: impl Fn(usize) -> usize,
) -> bool {
    groups(ip).into_iter().any(|(group, most)| {
        peers
            .clone()
            .filter(|peer| groups(*peer).iter().any(|(theirs, _)| *theirs == group))
            .count()
            >= bound(most)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("an address")
    }

    #[test]
    fn an_ipv6_address_counts_in_its_64_and_its_48_and_a_mapped_ipv4_one_is_itself() {
        let a = ip("2001:db8:1:2:aaaa::1");
        assert_eq!(source(a), source(ip("2001:db8:1:2:ffff:1:2:3")));
        assert_ne!(source(a), source(ip("2001:db8:1:3::1")));
        assert_eq!(groups(a)[1].0, groups(ip("2001:db8:1:3::1"))[1].0);
        assert_ne!(groups(a)[1].0, groups(ip("2001:db8:2:2::1"))[1].0);
        // A /64 whose fourth group is zero is still not its /48.
        let zero = groups(ip("2001:db8:1::1"));
        assert_ne!(zero[0].0, zero[1].0);
        assert_eq!(source(ip("::ffff:192.0.2.7")), source(ip("192.0.2.7")));
    }

    #[test]
    fn a_place_is_refused_past_any_bound_of_its_groups_and_given_back_on_drop() {
        let places = Places::default();
        let held: Vec<Place> = (0..MAX_PER_IPV6_48)
            .map(|n| {
                places
                    .take(ip(&format!("2001:db8:1:{n:x}::1")))
                    .expect("room in the /48")
            })
            .collect();
        assert!(
            places.take(ip("2001:db8:1:ffff::1")).is_none(),
            "the /48 is full"
        );
        assert!(
            places.take(ip("2001:db8:2::1")).is_some(),
            "another /48 is not"
        );
        drop(held);
        assert!(
            places.take(ip("2001:db8:1:ffff::1")).is_some(),
            "given back"
        );
    }
}
