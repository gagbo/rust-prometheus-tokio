# Prometheus + Tokio

This project exposes a Tokio collector that is compatible with the
[prometheus](https://github.com/tikv/rust-prometheus) crate that adds tokio
runtime related metrics.

The project relies on a `tokio_unstable` feature, so you need to add the flags
accordingly, e.g. by adding a `.cargo/config.toml` file to your project with the
correct RUSTFLAGS, as is done [here](.cargo/config.toml).

You can quickly test to look at the metrics that exist by using the example app:

``` sh
cargo run --example complete-setup
# In another terminal
curl 127.0.0.1/metrics
```

## Acknowledgement

The project was kickstarted by [Fiberplane](https://fiberplane.com) during a "Hack day".
