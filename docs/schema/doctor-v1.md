# doctor-v1

Doctor JSON reports checks with stable ids, severity, status, summary, details, and remediation hints for adapters, permissions, and runtime capabilities.

PLAN-11 adds optional `subsystems.events` fields without changing the schema id. `dropped` is sourced from a shared `EventDropCounter` when a long-lived event fan-in is available. Stateless CLI doctor runs cannot observe a historical fan-in, so they report `drop_counter_observable: false` with `drop_counter_source: "unavailable_in_stateless_cli_doctor"` and keep `dropped` at `0`.
