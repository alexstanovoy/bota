# bota_gym

Train and evaluate bots against [bota](../../README.md) in your own process: many deterministic matches stepped
together on a pool of workers, a seeding scheme that replays runs exactly, opponent scheduling, evaluation
tournaments and per-seat statistics. Your policy decides; the gym knows no neural network or training algorithm.

```sh
cargo run --release -p bota_gym --example random_duel
cargo run --release -p bota_gym --example privileged_read
```

The crate docs are the guide: `cargo doc -p bota_gym --open`.
