//! Opt-in live-stack integration test for the TensorZero backend.
//!
//! Skips silently when `LOKER_TZ_INTEGRATION` is unset or empty so that plain
//! `cargo test` (developer machines and CI) does not require a running
//! TensorZero gateway. When the gate is set, drives one end-to-end round-trip
//! through `TensorZeroBackend::query` against a local Tier-2 stack and asserts
//! a structurally valid response.
//!
//! Mocked wire-contract coverage lives in `tests/tensorzero_backend.rs`
//! (CLO-248). This file exists to catch drift between that mock and the real
//! gateway on the parts wiremock cannot exercise: auth-header propagation,
//! the `genai`-OpenAI adapter path actually reaching `/openai/v1/chat/completions`,
//! and the `genai` layer's extraction of assistant content + token usage from
//! the response (D1 §2-§4, `docs/spikes/2026-04-25-tensorzero-roundtrip.md`).
//! It does **not** assert the gateway's echoed `model` field
//! (`tensorzero::function_name::<fn>::variant_name::<v>`) because
//! `TensorZeroBackend::query` overwrites `QueryOutput.model` with the effective
//! input (the configured function name) before returning - validating the echo
//! shape would require exposing it as a separate field.
//!
//! Run instructions: see `docs/handoff.md` ("How to run the live integration
//! test"). TL;DR:
//!
//! ```sh
//! cd tensorzero && docker compose up -d
//! LOKER_TZ_INTEGRATION=1 cargo test --test tensorzero_integration
//! ```

use std::env;
use std::path::Path;
use std::time::Duration;

use loker::backend::{Backend, TensorZeroBackend, TensorZeroBackendOpts};

const DEFAULT_GATEWAY_URL: &str = "http://localhost:3000";
const DEFAULT_FUNCTION: &str = "loker_d1_openai";
const PROMPT: &str = "Reply with the single word: pong.";
const TIMEOUT: Duration = Duration::from_secs(5);

fn integration_enabled() -> bool {
    env::var("LOKER_TZ_INTEGRATION")
        .ok()
        .filter(|v| !v.is_empty())
        .is_some()
}

fn gateway_url() -> String {
    env::var("TENSORZERO_GATEWAY_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_GATEWAY_URL.to_string())
}

fn function_name() -> String {
    env::var("LOKER_TZ_INTEGRATION_FUNCTION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_FUNCTION.to_string())
}

fn api_key() -> Option<String> {
    env::var("TENSORZERO_API_KEY")
        .ok()
        .filter(|v| !v.is_empty())
}

#[tokio::test]
async fn tz_integration_round_trip_via_loker_d1_openai() {
    if !integration_enabled() {
        return;
    }

    let function = function_name();
    let opts = TensorZeroBackendOpts {
        endpoint: gateway_url(),
        model: function.clone(),
        api_key: api_key(),
        timeout: TIMEOUT,
    };

    let backend = TensorZeroBackend::new(opts).expect("TensorZeroBackend::new failed");

    let out = backend
        .query(PROMPT, Path::new("."), None)
        .await
        .unwrap_or_else(|err| {
            panic!("LOKER_TZ_INTEGRATION round trip failed: {err}");
        });

    assert_eq!(
        out.backend, "tensorzero",
        "expected backend name 'tensorzero', got {:?}",
        out.backend
    );
    assert!(
        !out.stdout.is_empty(),
        "gateway returned empty assistant content"
    );

    // `TensorZeroBackend::query` populates `QueryOutput.model` from the
    // *effective input* (the configured function name / per-call override),
    // not the gateway's echoed response `model` field. Pin that contract so a
    // future change that switches to propagating the response echo trips this
    // assertion deliberately.
    let model = out.model.as_deref().expect("response model field was None");
    assert_eq!(
        model, function,
        "expected QueryOutput.model to equal the effective input function {function:?}, got {model:?}"
    );

    let usage = out.usage.as_ref().expect("usage block missing");
    assert!(
        usage.prompt_tokens > 0,
        "expected prompt_tokens > 0, got {}",
        usage.prompt_tokens
    );
    assert!(
        usage.completion_tokens > 0,
        "expected completion_tokens > 0, got {}",
        usage.completion_tokens
    );

    assert!(
        out.duration > Duration::ZERO,
        "expected duration > 0, got {:?}",
        out.duration
    );
}
