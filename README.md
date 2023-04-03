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

## Forced feature

Due to feature unification, note that bringing this crate in your project will
force the `rt` feature of `tokio` in your project. This should _not_ be an
issue, given that runtime metrics hopefully make little sense if there is no
runtime to measure.

## Features

The crate has a single feature:

- `io_driver` enables the `tokio/net` feature, and adds an extra metric that
  reports the number of ready events processed by the runtime’s I/O driver.

## Acknowledgement

The project was kickstarted by [Fiberplane](https://fiberplane.com) during a "Hack day".
