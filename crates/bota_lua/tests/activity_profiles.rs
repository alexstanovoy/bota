use bota_client_core::ActivityProfile;
use bota_lua::{Limits, ModuleBank};

#[test]
fn trusted_activity_profiles_reject_invalid_keys_durations_and_storage() {
    let profile = ActivityProfile {
        idle_ms: Some(600_000),
        ..Default::default()
    };
    let mut bank = ModuleBank::new(Limits::DEFAULT);
    bank.set_activity_profile("npc_test", profile)
        .expect("inclusive duration boundary");
    assert!(
        bank.set_activity_profile("npc_test", profile).is_err(),
        "duplicate"
    );
    for key in ["", "../npc_test", "npc/test", "npc-test"] {
        assert!(bank.set_activity_profile(key, profile).is_err());
    }
    let too_long = "x".repeat(257);
    assert!(bank.set_activity_profile(&too_long, profile).is_err());
    for profile in [
        ActivityProfile {
            idle_ms: Some(600_001),
            ..Default::default()
        },
        ActivityProfile {
            channel_ms: [Some(u32::MAX); 6],
            ..Default::default()
        },
    ] {
        assert!(bank.set_activity_profile("npc_bad", profile).is_err());
    }
    let mut tiny = ModuleBank::new(Limits::DEFAULT.with_memory(1024));
    assert!(
        tiny.set_activity_profile("npc_test", profile).is_err(),
        "storage charged before insertion"
    );
    let mut bank = ModuleBank::new(Limits::DEFAULT);
    for n in 0..1024 {
        bank.set_activity_profile(&format!("npc_{n}"), ActivityProfile::default())
            .expect("bounded profile");
    }
    assert!(
        bank.set_activity_profile("npc_extra", ActivityProfile::default())
            .is_err()
    );
}
