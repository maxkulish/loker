The refactor largely works, but it regresses the verification API by dropping `QueryOutput::usage`, and the new `FailureReason` builder API does not implement the truncation behavior its contract promises. Those issues make the new types incomplete as a replacement for the old interface.

Full review comments:

- [P2] Keep token usage when converting `QueryOutput` into `VerifyContext` — /Users/mk/Code/orchestrator/loker--feat-clo-270-hook/src/strategy/verify.rs:222-230
  When a verify hook needs token/cost metadata, the new `VerifyContext` is no longer a full replacement for the old `&QueryOutput`: `QueryOutput::usage` is dropped here and there is no field on `VerifyContext` to recover it later. That means any cost-aware hook will require another breaking API change even though the information is already present on the backend result.

- [P3] Enforce `FailureReason` output truncation in the builder API — /Users/mk/Code/orchestrator/loker--feat-clo-270-hook/src/strategy/verify.rs:66-75
  If a verifier attaches real stdout/stderr through these builders, the full strings are stored verbatim and `truncated` stays whatever the caller set manually. That contradicts the type contract in this file (`stdout`/`stderr` capped with a truncation flag) and means a single failing verifier can still carry arbitrarily large logs into memory or later prompt context unless every hook reimplements the cap itself.