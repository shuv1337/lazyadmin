# Metrics panel decision

Decision: rates are derived from differences between consecutive snapshots, not an EWMA and not extra `/proc` polling.

The metrics panel is situational awareness, not monitoring. Counts come from snapshot data: listener exposure, workload runtime, warning severity, tracked runs, event drops, and a small in-memory event-rate ring buffer for display.
