//! Compiled against the replaced build, so both its greeting and error wording use that build.
fn main() {
    let argument = std::env::args()
        .nth(1)
        .expect("address or --wire-fingerprint");
    if argument == "--wire-fingerprint" {
        println!("{:016x}", bota_core::WIRE_FINGERPRINT);
        return;
    }
    match bota_client_core::Link::join_with_timeout(
        &argument,
        "previous client",
        bota_proto::HeroId::SHADOW_FIEND,
        None,
        std::time::Duration::from_secs(30),
    ) {
        Ok(_) => std::process::exit(0),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
