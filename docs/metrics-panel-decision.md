# Metrics panel decision

Decision: rates are derived from differences between consecutive snapshots, not an EWMA and not extra `/proc` polling.

The metrics panel is situational awareness, not monitoring. Counts come from snapshot data: listener exposure, workload runtime, warning severity, tracked runs, and event drops. v0.2 exposes only coarse non-negative snapshot-diff samples for rate display; per-adapter latency/throughput and telemetry-backed sparklines are deferred.
