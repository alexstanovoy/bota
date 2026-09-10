use clap::{Parser, error::ErrorKind};

use crate::Args;

#[test]
fn map_cli_accepts_each_registered_map() {
    for id in ["0", "1", "2"] {
        let args = Args::try_parse_from(["bota-server", "--map", id]).expect("registered map");

        assert_eq!(args.map.to_string(), id);
    }
}

#[test]
fn map_cli_rejects_unknown_maps_with_supported_ids_in_the_error() {
    for id in ["3", "65535", "not-a-map", "-1"] {
        let error = Args::try_parse_from(["bota-server", &format!("--map={id}")])
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
    let args = Args::try_parse_from(["bota-server"]).expect("default arguments");

    assert_eq!(args.map, 0);
}
