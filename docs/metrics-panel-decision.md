# Metrics panel decision

Decision: rates are derived from differences between consecutive snapshots, not an EWMA and not extra `/proc` polling.

The metrics panel is situational awareness, not monitoring. Counts come from snapshot data: listener exposure, workload runtime, warning severity, tracked runs, and event drops. v0.2 exposes coarse non-negative snapshot-diff samples for rate display and a thin in-memory event ring for per-adapter throughput, drop counts, and sparklines. Adapter latency is reserved in the view model and shown as unavailable until adapters report timed health samples.
