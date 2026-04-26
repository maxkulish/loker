# `loker ui` Threat Model (D4)

| Field | Value |
|-------|-------|
| Author | Max Kulish |
| Status | Accepted (closes PRD §11 D4) |
| Created | 2026-04-25 |
| Scope | M10 / M11 - localhost-bound axum UI for HITL gates |
| Audience | Implementers of M10 / M11 and the M11 threat-model test suite (T-055) |
| Cross-refs | PRD §5 (Security NFRs), PRD §8 (risk row "Localhost UI threat surface"), PRD §11 D4, HITL design §4-§5, FR-26, FR-28 |

## 1. Scope and assumptions

This document covers the **Phase 1** posture defined in HITL design §5: localhost-bound bind, no authentication, anyone with a shell on the host can submit. v0 ships Phase 1 only. Phase 2 (Google OAuth allowlist) is explicitly deferred and noted where it changes the calculus.

**In scope**:

- Routes enumerated in HITL design §4.3 (`GET /runs`, `GET /runs/<id>`, `GET /runs/<id>/trace.sse`, `GET /runs/<id>/artefact/<path>`, `POST /runs/<id>/responses/<phase>`, `POST /runs/<id>/responses/<phase>/inline`, `GET /healthz`).
- Both `loker ui` (one-shot per-gate) and `loker ui --serve` (daemon) - they share 100% of route handlers per FR-27, so the threat surface is identical.
- The advisory lock protocol (`<phase>.json.lock`) per FR-26.

**Trust boundaries**:

| Boundary | Treatment |
|----------|-----------|
| Network -> host | Closed by `127.0.0.1` bind. A non-localhost bind would re-open this and is gated on Phase 2 auth (see §6). |
| Host user -> loker process | Trusted in Phase 1. Same-machine attacker with the user's UID can already read `runs/`, the kernel keyring, and `~/.config`; the UI adds no new exposure relative to the shell. |
| Browser tab on the user's host -> loker process | **Primary defended boundary in v0.** A page from `evil.com` can attempt cross-origin requests to `127.0.0.1:<port>`; this is what the mitigations target. |
| Browser extension -> loker process | Accepted risk. Documented mitigation is operational (separate browser profile), not technical. See T7. |
| Filesystem of `runs/<id>/` -> loker process | Trusted to be writable by the loker run only, but symlink and traversal handling cannot assume that. See T3, T4. |

**Out of scope**:

- Memory hygiene and secret redaction inside the loker process (covered by PRD §5 rows 193-196).
- Supply-chain attacks on Cargo dependencies (covered by PRD §8 risk row "LiteLLM-style supply chain compromise").
- DoS resistance under sustained malicious local load. Same-user attacker can already `rm -rf runs/`; UI hardening here is not load-bearing.
- TLS / reverse-proxy deployments. Out of v0; if the user fronts loker with a reverse proxy on a non-loopback interface, Phase 2 auth is required (§6).

## 2. Attacker model

| ID | Adversary | Capabilities | Defended? |
|----|-----------|--------------|-----------|
| A1 | Cross-origin web page (drive-by) | Render HTML in the user's browser; issue cross-origin GET / POST / fetch / `<form>` / `<img>` / `<script>` / `<iframe>` to `127.0.0.1:<port>` | Yes (primary) |
| A2 | Stale or runaway browser tab holding a phase lock | Hold `<phase>.json.lock` after the user has walked away; never heartbeat | Yes (FR-26 timeout) |
| A3 | Filesystem state attacker (compromised CI, shared tmp, accidentally-followed symlink) | Plant symlinks or `..` paths inside `runs/<id>/` | Yes |
| A4 | Cookie-reflection / state-leakage attacker | Trick the browser into reflecting cookies set by a GET into a forged POST (CSRF via cookie reflection, per task description) | Yes (no cookies set in v0) |
| A5 | Malicious browser extension | Read DOM and network on any visited page including `127.0.0.1`; exfiltrate to attacker server | **Accepted risk in Phase 1** - operational mitigation only |
| A6 | Same-user local process | Connect directly to `127.0.0.1:<port>` and exercise any handler | **Accepted** - same trust boundary as the shell |
| A7 | Remote network attacker | Reach loker over a network interface | Closed by `127.0.0.1` bind (FR-28) |

**Why A1 is the primary defended adversary**: localhost-bind closes A7, the same-user trust model accepts A6, and A5 cannot be solved without removing the browser. That leaves the cross-origin tab as the only adversary the UI can both meaningfully attack and meaningfully defend, and it is the realistic one (every user with loker installed has a browser, and the browser visits other sites).

## 3. Threats and mitigations

Each threat below maps to one Security NFR row in PRD §5 and to one or more concrete tests in §5 of this document. Tests ship with M11 as T-055.

### T1. Cross-origin POST (CSRF)

**Description**: A page on `evil.com` renders an autosubmitting form that POSTs to `http://127.0.0.1:<port>/runs/<id>/responses/<phase>` with body `verdict=approve`. The browser issues the request from the user's session; without defenses, the loker process accepts it and writes the response file.

**Mitigation** (PRD §5 row 198 - "origin match for state-changing POSTs"):

1. Every POST handler asserts the request `Origin` header equals `http://127.0.0.1:<port>` (or `http://localhost:<port>`); reject with `403 Forbidden` on mismatch or absence.
2. As defense-in-depth, also assert `Referer` is absent or matches the same origin.
3. Reject POST with `Content-Type: text/plain` or `multipart/form-data` (both are CORS "simple" content types that bypass browser preflight and are usable for CSRF; `application/x-www-form-urlencoded` is also "simple" but is needed by the legitimate form). Require `application/x-www-form-urlencoded` or `application/json`.
4. No CORS headers emitted. The default same-origin policy is the desired posture.

**Why this is sufficient**: All major browsers (Chrome 90+, Firefox 90+, Safari 16+) attach an `Origin` header on cross-origin POSTs including `<form>` submissions. A request without a recognised `Origin` is either same-origin (acceptable) or non-browser (where the same-user trust boundary applies, A6).

### T2. Cross-origin read of artefact / SSE

**Description**: A cross-origin page embeds `<img src="http://127.0.0.1:<port>/runs/<id>/artefact/foo">` or opens an `EventSource` to `/trace.sse` and tries to read trace events.

**Mitigation**:

1. `Cross-Origin-Resource-Policy: same-origin` on every response from `/runs/...` - blocks cross-origin embedding via `<img>`, `<script>`, `<link>`, etc.
2. `X-Content-Type-Options: nosniff` on artefact responses - prevents MIME sniffing that could turn a markdown file into executable HTML.
3. SSE endpoint emits no `Access-Control-Allow-Origin`; cross-origin `EventSource` is therefore blocked by the browser without our doing.
4. Artefact handler serves unknown / non-allowlisted extensions with `Content-Type: text/plain; charset=utf-8` and `Content-Disposition: attachment` - the page's content is never rendered as HTML in the loker origin.

**Test rationale**: opaque cross-origin fetches still leak existence-by-timing; this is acceptable in Phase 1 (run IDs are not secret to anyone with shell access).

### T3. Path traversal

**Description**: An attacker (cross-origin tab via T1, or a malicious local process via A6) issues `GET /runs/<id>/artefact/../../../etc/passwd`, hoping the artefact handler concatenates the path.

**Mitigation** (PRD §5 row 198 - "refuse path traversal"):

1. Nested artefact paths are allowed. Percent-decode the captured `<path>` first, split it on `/` into components, then validate each decoded component individually. Reject any component that is empty, `.`, `..`, contains a NUL byte, contains `\` (Windows-style separator), or is otherwise not a normal path segment on the target platform. `/` is the separator that produced the component list, so it is not itself a per-component check.
2. Reject absolute paths in the URL parameter outright (a leading `/` after percent-decoding, or any platform-specific rooted form such as `C:\` or `\\?\` on Windows).
3. After component validation, build the candidate path by joining the validated component list under `runs/<id>/`; never `path.join(user_input)` on the raw captured string.
4. Pass the candidate path to T4 (symlink) validation. Only after both T3 and T4 succeed does the handler open the file.

### T4. Symlink escape

**Description**: A symlink inside `runs/<id>/` (planted by a malicious workflow phase, a compromised CI step, or an adversarial filesystem layout) points to `/etc/passwd`. The artefact handler follows it and serves the target.

**Mitigation** (PRD §5 row 198 - "follow no symlinks outside the run directory"):

1. **Walk the *unresolved* candidate path first.** After T3 produces the validated component list, walk each ancestor under `runs/<id>/` and call `tokio::fs::symlink_metadata` on it. If any component (intermediate or leaf) is a symlink, refuse the read. This must happen *before* canonicalisation, because `canonicalize` resolves symlinks and the resolved path no longer carries the link components. An `openat`/`O_NOFOLLOW`-per-component traversal is an acceptable equivalent if it enforces the same property for every component, but plain `open` with `O_NOFOLLOW` is **not** sufficient (it only protects the final component).
2. **Then canonicalise and prefix-check.** After the unresolved-path symlink walk passes, call `tokio::fs::canonicalize` on the candidate path and assert the canonicalised result begins with the canonicalised `runs/<id>/` root. Reject otherwise. This is defence-in-depth against any race between the walk and the open, and against TOCTOU on platforms where `symlink_metadata` and `open` are not atomic.
3. **Open via tokio.** Use `tokio::fs::File::open` (not `std::fs`) inside the axum handler so the blocking syscall does not stall the runtime. Do not rely on `O_NOFOLLOW` alone.
4. Trace events emitted by the workflow itself never traverse symlinks; the engine writes artefacts via `<tmp> -> rename` per design doc D3.

**Why walk the unresolved path and not just the canonicalised one**: `canonicalize` resolves every symlink along the way. A symlink at `runs/<id>/intermediate/` pointing to `/etc/` would canonicalise `runs/<id>/intermediate/passwd` to `/etc/passwd`, which a prefix check correctly rejects - but if the symlink pointed *inside* `runs/<id>/` (a self-relative loop, or a link to a sibling run), the prefix check would pass and we would silently follow a link the policy forbids. Walking the unresolved path catches that case.

### T5. Stale-lock takeover (FR-26)

**Description**: A reviewer claims the phase lock, then closes their laptop without resolving the gate. Without auto-release, the workflow stalls forever; with naive auto-release, two reviewers race and the second can overwrite a still-valid response file.

**Mitigation** (FR-26 - "first-write-wins per-phase advisory lock with 60s heartbeat auto-release"):

1. `<phase>.json.lock` is created with `O_CREAT | O_EXCL`. The contents include `{ session_id, claimed_by, last_heartbeat_at }`.
2. The owning client `PUT`s a heartbeat every 20 s.
3. Any other client that finds the lock with `last_heartbeat_at` older than 60 s deletes it and re-claims; the new owner's `session_id` differs from the old one's.
4. **First-write-wins**: when a POST writes the response file, the lock check is best-effort but not the source of truth. Once `responses/<phase>.json` exists, all further POSTs to the same phase return `409 Conflict` regardless of lock state.
5. The lock file itself never receives state-changing POSTs from the browser; it is private to the route handlers.

**Risk acknowledgement**: a malicious local process (A6) can spam lock claims to DoS the legitimate UI client. Accepted - same-user trust boundary applies, and `rm -rf runs/` is a strictly worse attack the same actor can already perform.

### T6. CSRF via cookie reflection

**Description** (verbatim from the task scope): a malicious origin sets or reflects a session cookie via a GET to a benign endpoint, then triggers a POST that reuses it.

**Mitigation**:

1. **v0 sets no cookies at all.** FR-28 closes this threat by construction. There is no session state to reflect.
2. Phase 2 (deferred): when OAuth lands, the session cookie must be `Secure; HttpOnly; SameSite=Strict; Path=/`. The `Authenticator` swap point in HITL design §5 is where this enforcement lives.

### T7. Browser extension snooping

**Description**: A malicious extension with `<all_urls>` host permission reads the page DOM, intercepts the SSE trace, or injects a forged response POST.

**Mitigation** (operational, accepted risk in Phase 1):

1. Set `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; frame-ancestors 'none'; base-uri 'self'`. This does **not** stop a permissioned extension - extensions run in a privileged isolated world - but it does limit damage from any inadvertently-injected inline script in a template.
2. `X-Frame-Options: DENY` on every HTML response (also covered by `frame-ancestors 'none'` above; both are emitted for older browsers).
3. **Operational guidance** documented in the M11 README: run `loker ui` against a separate browser profile with no extensions, or use SSH port-forwarding from a hardened machine.
4. No technical defense is added beyond CSP; the residual risk is documented and the phase 2 gate (OAuth + per-user attribution) is the real fix.

### T8. Open redirect / link-injection

**Description**: A handler accepts a user-supplied URL parameter and 302-redirects, allowing `/redirect?url=evil.com` to be used for phishing.

**Mitigation**: no handler in HITL design §4.3 takes a redirect target as input. The only redirect is `/` -> `/runs/<most-recent-id>`, computed server-side from filesystem state. A snapshot test of the route table guards this invariant.

## 4. Decision: per-session URL token vs origin checks

PRD §11 D4 asks: *"Decide whether v0 needs a per-session token in the URL or whether origin checks plus localhost bind are sufficient."*

**Verdict**: origin checks plus localhost bind are sufficient for v0. Do not ship URL tokens.

**Reasoning**:

1. **Tokens do not defend against the residual adversary.** The two adversaries that origin-checks-plus-localhost-bind do not handle are A5 (extensions) and A6 (same-user local processes). An extension can read the URL, and a local process does not need to guess a token because it can read `runs/<id>/` directly. URL tokens add complexity without buying defense against either.
2. **Tokens leak via Referer.** A `loker ui` page that contains an external link (the user clicks through to a docs URL) leaks the token to the destination unless `Referrer-Policy: no-referrer` is set. We would need that policy anyway, and once we have it, the residual benefit of the token is marginal.
3. **Tokens hurt UX without a corresponding security gain.** Users copy URLs into tickets, share them with collaborators on the same machine, refresh after daemon restarts. Each of those is a paper cut that an origin check does not impose.
4. **Phase 2 makes tokens obsolete.** When OAuth lands, identity is per-user, not per-tab. Designing v0 around URL tokens creates a migration we then unwind.

**What this commits to**:

- Every state-changing handler validates `Origin` (T1).
- Every response carries `Referrer-Policy: no-referrer` (covers token-via-Referer leak even though we ship no token).
- Phase 2 OAuth design decides cookie strategy from scratch; URL tokens never enter the code path.

## 5. Test list (M11 / T-055)

Each test below is a concrete fixture for the M11 threat-model test suite. The convention `T-<topic>-<n>` is the test name used in the M11 integration suite.

| Test ID | What it asserts | Maps to threat | Maps to PRD §5 NFR |
|---------|------------------|-----------------|---------------------|
| T-CSRF-1 | `POST /runs/<id>/responses/<phase>` with `Origin: http://evil.com` returns `403`; no response file written | T1 | row 198 (origin match) |
| T-CSRF-2 | POST without `Origin` header returns `403` (defense in depth) | T1 | row 198 |
| T-CSRF-3 | POST with `Origin: http://127.0.0.1:<port>` succeeds and writes the response file | T1 | row 198 (positive case) |
| T-CSRF-4 | POST with `Content-Type: text/plain` or `multipart/form-data` returns `415` regardless of Origin (covers both CORS "simple" content types that bypass preflight) | T1 | row 198 |
| T-XFRAME-1 | Every HTML response includes `X-Frame-Options: DENY` and `Content-Security-Policy` with `frame-ancestors 'none'` | T7 | row 198 (X-Frame-Options) |
| T-CORP-1 | `GET /runs/<id>/artefact/<path>` includes `Cross-Origin-Resource-Policy: same-origin` and `X-Content-Type-Options: nosniff` | T2 | row 198 |
| T-MIME-1 | Unknown-extension artefact served as `Content-Type: text/plain; charset=utf-8` with `Content-Disposition: attachment` | T2 | row 198 |
| T-TRAVERSAL-1 | `GET /runs/<id>/artefact/../../../etc/passwd` returns `400`; reads no file outside the run root | T3 | row 198 |
| T-TRAVERSAL-2 | Percent-encoded traversal (`%2e%2e%2f`) returns `400` after decode | T3 | row 198 |
| T-TRAVERSAL-3 | Absolute-path artefact param (`/etc/passwd`) returns `400` | T3 | row 198 |
| T-SYMLINK-1 | Symlink at `runs/<id>/escape -> /etc/passwd`, GET `/artefact/escape` returns `403`; the unresolved-path symlink walk (T4 step 1) refuses the read before canonicalisation, and the post-canonicalise prefix check would also reject the resolved `/etc/passwd` | T4 | row 198 |
| T-SYMLINK-2 | Intermediate-component symlink (`runs/<id>/dir -> /tmp`, file `foo` inside) is refused even though the leaf `foo` is a regular file; the policy is "no symlinks anywhere along the unresolved path", not just at the leaf | T4 | row 198 |
| T-LOCK-1 | Client A claims lock, never heartbeats; advance test clock 70 s; client B claims successfully | T5 | FR-26 |
| T-LOCK-2 | Two simultaneous claimants: second sees `read-only` UI state; only the holder can submit | T5 | FR-26 |
| T-LOCK-3 | After `responses/<phase>.json` exists, further POSTs to the same phase return `409` regardless of lock state (first-write-wins) | T5 | FR-26 |
| T-COOKIE-1 | No `Set-Cookie` header on any response in v0 | T6 | row 197 (no auth in v0) |
| T-CSP-1 | CSP header includes `default-src 'self'; script-src 'self'; frame-ancestors 'none'; base-uri 'self'` and no `'unsafe-inline'` | T7 | row 198 |
| T-REFERRER-1 | `Referrer-Policy: no-referrer` present on every response | §4 decision | row 198 |
| T-BIND-1 | Default bind address is `127.0.0.1`; explicit non-localhost bind via flag emits a `WARN` log line referencing this doc | A7 | row 197 |
| T-METHOD-1 | GET-only routes return `405 Method Not Allowed` on POST; POST-only routes return `405` on GET | T1 (defense in depth) | n/a |
| T-REDIRECT-1 | Snapshot test of route table confirms no handler accepts a user-controlled redirect target | T8 | row 198 |

**Test mechanics**:

- All tests run inside `tests/ui/threat_model.rs` (created in M11). Each test starts an axum app on an ephemeral port, exercises the handler with `reqwest`, and asserts response status, headers, and side effects on the run directory.
- Symlink tests use a `tempfile::TempDir` populated by the test fixture; CI runs on macOS and Linux per PRD §5 Portability NFR.
- Lock tests use the test-only fake clock injected at app construction (same mechanism M2 uses for `EscalatingRetry` timeout tests).

## 6. Out of scope for v0 / deferred to Phase 2

| Concern | v0 stance | Phase 2 plan |
|---------|-----------|--------------|
| OAuth allowlist authentication | Not implemented; localhost-bind is the only access control | Google OAuth via GCP (HITL design §5); `Authenticator` trait swap |
| Per-user identity in `responses/<phase>.json` (`claimed_by` field) | Recorded as `"anon"` in v0 | Recorded as the OAuth-verified email |
| Non-loopback bind | Permitted via `--bind` flag with a WARN log; security is on the operator | OAuth + TLS termination at a reverse proxy required |
| Browser-extension defense | Operational only - separate browser profile recommended | OAuth raises the bar (extension still has user's session, but per-user audit limits damage) |
| TLS termination | Not in v0 | Documented reverse-proxy recipe; loker stays plaintext-on-loopback |
| Rate limiting / DoS | Same-user trust boundary makes this moot | Reconsidered if Phase 2 lifts the localhost restriction |

## 7. Open questions deferred to implementation

1. **CSP `style-src` and HTMX vendored asset**: HTMX is vendored via `rust-embed`, so `script-src 'self'` covers it. If a future template needs an inline style attribute, decide between `style-src 'self' 'unsafe-inline'` vs hashed inline. Owner: M11 implementer. Deadline: M11 close.
2. **WebSocket vs SSE for trace stream**: HITL design §4.3 commits to SSE. If a future need pushes us to WebSocket, the same-origin posture must be re-verified - WebSocket does not honour CORS-like preflight checks the same way. Owner: M11 implementer. Deadline: post-v0.
3. **HSTS / loopback over HTTPS**: out of v0; if a Phase 2 deployment fronts loker with TLS, HSTS becomes meaningful. Owner: Phase 2 design. Deadline: Phase 2 close.

## 8. References

- PRD §5 (Security NFRs), rows 197-198 - the rows this document closes the loop on.
- PRD §8 risk row "Localhost UI threat surface" - the risk this document mitigates.
- PRD §11 D4 - the discovery item this document is the deliverable for.
- HITL design §4.3 Routes, §4.4 First-write-wins concurrency, §5 Authentication.
- FR-26 (advisory lock with 60 s heartbeat), FR-27 (one-shot / daemon share handlers), FR-28 (localhost-bound, no auth in v0).
- M11 test contract (HITL design §6 M11 - Test contract). T-055 is the threat-model test suite this document defines.
