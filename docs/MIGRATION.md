# MineRadio Sidecar API — Remaining Migration Work

The TypeScript-to-Rust implementation work is complete. This document keeps only
the remaining cross-cutting validation work; provider-specific compatibility gaps
are tracked in [PROVIDER_TS_AUDIT.md](PROVIDER_TS_AUDIT.md).

## Remaining work

- [ ] Add HTTP-level end-to-end tests with mocked external APIs.
- [ ] Compare Rust performance with the historical Bun sidecar under representative
  workloads.
- [ ] Verify that all TypeScript regression scenarios that matter to supported
  clients have Rust coverage.

## Validation approach

- Keep focused unit tests beside each Rust module.
- Use HTTP-level tests for route wiring and response envelopes.
- Use mock servers for external provider responses; perform live-provider checks
  separately because those services are stateful and change over time.
