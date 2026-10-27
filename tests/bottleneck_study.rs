// Exercise the standalone, safe-std diagnostic in every ordinary CI mode.
// Its standalone main is not called by the integration-test harness.
#[allow(dead_code)]
#[path = "../benchmarks/studies/current-bottlenecks/probe.rs"]
mod probe;
