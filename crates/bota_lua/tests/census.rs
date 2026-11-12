#![cfg(feature = "native")]

#[test]
fn installed_api_catalogue_is_available_without_pack_or_connection() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bota_botscript"))
        .arg("--api-catalogue")
        .output()
        .expect("catalogue process");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let catalogue = String::from_utf8(output.stdout).expect("UTF-8 catalogue");
    for entry in ["global\tGetBot\n", "unit\tGetHealth\n", "ability\tGetSpecialValueInt\n", "math\trandom\n"] {
        assert!(catalogue.lines().any(|line| line == entry.trim_end()), "missing {entry:?}");
    }
    assert!(!catalogue.contains("\tMissingProbe\n"));
}
