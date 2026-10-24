use clap::{Parser, error::ErrorKind};

use crate::Args;

#[test]
fn map_cli_accepts_each_registered_map() {
    for id in ["0", "1", "2"] {
        let args = Args::try_parse_from(["bota_server", "--map", id]).expect("registered map");

        assert_eq!(args.map.to_string(), id);
    }
}

#[test]
fn map_cli_rejects_unknown_maps_with_supported_ids_in_the_error() {
    for id in ["3", "65535", "not-a-map", "-1"] {
        let error = Args::try_parse_from(["bota_server", &format!("--map={id}")])
            .expect_err("unknown maps must not fall back to Map0");

        assert_eq!(error.kind(), ErrorKind::ValueValidation);
        assert!(
            error
                .to_string()
                .contains(&format!("unsupported map '{id}'"))
        );
        assert!(
            error
                .to_string()
                .contains("expected 0 (Dota), 1 (demo), or 2 (mid-only Dota)")
        );
    }
}

#[test]
fn map_cli_default_stays_map0() {
    let args = Args::try_parse_from(["bota_server"]).expect("default arguments");

    assert_eq!(args.map, 0);
}

#[test]
fn objectives_and_runes_are_sets_by_name_and_what_a_map_does_not_play_fails_before_ready() {
    use bota_proto::{ObjectiveKind, ObjectiveSet, RuneSet};
    let args = Args::try_parse_from(["bota_server", "--objectives", "outposts,shrines"])
        .expect("objective names");
    assert_eq!(
        args.objectives,
        Some(ObjectiveSet::of(ObjectiveKind::Outposts).with(ObjectiveKind::Shrines))
    );
    assert_eq!(args.runes, None, "absent: the map's default");
    let none = Args::try_parse_from(["bota_server", "--map", "2", "--objectives", "none"])
        .expect("none is a set");
    assert_eq!(none.objectives, Some(ObjectiveSet::NONE));
    assert!(crate::check_opts(&crate::server_opts(&none).expect("options")).is_ok());
    for bad in [["--objectives", "towers"], ["--runes", "gold"]] {
        let error =
            Args::try_parse_from(["bota_server", bad[0], bad[1]]).expect_err("no such kind");
        assert_eq!(error.kind(), ErrorKind::ValueValidation, "{bad:?}");
    }
    // Roshan is not played yet, nor is any rune: asked for, the server
    // refuses before it listens, on every map.
    for map in ["0", "1", "2"] {
        for asked in [["--objectives", "roshan"], ["--runes", "bounty"]] {
            let args = Args::try_parse_from(["bota_server", "--map", map, asked[0], asked[1]])
                .expect("a set by name");
            let opts = crate::server_opts(&args).expect("options");
            let refused = crate::check_opts(&opts).expect_err("not played");
            assert!(refused.contains(&format!("map {map}")), "{refused}");
        }
    }
    let all = Args::try_parse_from(["bota_server", "--runes", "all"]).expect("all is a set");
    assert_eq!(all.runes, Some(RuneSet::ALL));
}

#[test]
fn every_map_defaults_to_rules_it_plays_and_map2_to_none() {
    use bota_proto::{MapId, ObjectiveSet, RuneSet};
    for map in bota_core::MAPS {
        assert!(
            map.default_objectives
                .without(map.playable_objectives())
                .is_empty()
                && map.default_runes.without(map.playable_runes()).is_empty(),
            "map {}",
            map.id.0
        );
    }
    let mid = bota_core::map_of(MapId(2));
    assert_eq!(
        (mid.default_objectives, mid.default_runes),
        (ObjectiveSet::NONE, RuneSet::NONE),
        "the mid-only map stands no objective and spawns no rune unless asked"
    );
}
