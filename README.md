# Prometheus + Tokio

This project exposes a Tokio collector that is compatible with the [prometheus](https://github.com/tikv/rust-prometheus) crate
that adds tokio runtime related metrics.

The project relies on a `tokio_unstable` feature, so you need to add the flags accordingly

## Acknowledgement

The project was kickstarted by [Fiberplane](https://fiberplane.com) during a "Hack day".
