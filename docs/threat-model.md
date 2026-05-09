# loker UI Threat Model

**Status:** Accepted (M11 close gate)  
**Scope:** `loker ui --serve` and the one-shot HITL fallback server  
**Detailed model:** [`docs/security/2026-04-25-ui-threat-model.md`](./security/2026-04-25-ui-threat-model.md)

---

## 1. Trust boundaries

| Boundary | Treatment |
|----------|-----------|
| Network → host | Closed by `127.0.0.1` bind. Non-loopback bind emits a `WARN` log. |
| Host user → loker process | Trusted in v0 (same UID already owns the shell and `runs/`). |
| Browser tab → loker process | **Primary defended boundary.** Cross-origin requests are rejected via `Origin` validation, `CORP`, and `CSP`. |
| Browser extension → loker process | Accepted risk in v0. Operational mitigation: separate browser profile. |
| Filesystem → loker process | Symlinks and path-traversal attempts are rejected before file access. |

## 2. In-scope assets

- Run metadata exposed by `GET /runs` and `GET /runs/:id`.
- Artefacts under `runs/<id>/` served by `GET /runs/:id/artefact/:path`.
- SSE trace streams (`/runs/:id/trace/sse`).
- HITL gate decisions (`POST /gates/:run_id/:phase/approve|reject`).
- Advisory lock state (`locks/<phase>.lock`).

## 3. Out-of-scope

- Multi-tenant / shared-host deployments (Phase 2, deferred).
- Memory hygiene and secret redaction inside the loker process (covered by PRD §5).
- Supply-chain attacks on Cargo dependencies (covered by PRD §8).
- TLS / reverse-proxy deployments (documented but not enforced in v0).
- DoS resistance under sustained malicious local load.

## 4. Test coverage

Every mitigation is exercised by the automated threat-model test suite in `tests/ui_threat_model.rs` (integration tests) and supporting unit tests in `src/ui/artefact.rs` and `src/run_state/phase_lock.rs`. The suite runs as part of `make check` and gates M11 close.

| Threat | Test IDs |
|--------|----------|
| Cross-origin POST (CSRF) | `T-CSRF-1` … `T-CSRF-4` |
| Cross-origin read / embedding | `T-CORP-1`, `T-MIME-1` |
| Path traversal | `T-TRAVERSAL-1` … `T-TRAVERSAL-3` |
| Symlink escape | `T-SYMLINK-1`, `T-SYMLINK-2` (unit tests in `src/ui/artefact.rs`) |
| Stale-lock takeover | `T-LOCK-1` … `T-LOCK-3`, plus `stale_lock_by_ttl_is_reclaimable` and `stale_lock_with_dead_pid_is_reclaimable` in `src/run_state/phase_lock.rs` |
| CSRF via cookie reflection | `T-COOKIE-1` |
| SSE cross-origin | `T-SSE-CSRF` |
| Browser extension snooping | `T-CSP-1`, `T-XFRAME-1` |
| Non-loopback bind | `T-BIND-1` |
| Gate URL entropy | `T-ENTROPY-1` (run_id is the entropy source; no per-gate token in v0) |
| Method enforcement | `T-METHOD-1` |
| Referrer leakage | `T-REFERRER-1` |

See the detailed model for full attacker definitions, mitigations, and deferred Phase-2 work.
