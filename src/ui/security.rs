//! Security helpers for the loker UI daemon.
//!
//! Provides:
//! - `add_security_headers` — adds CSP, X-Frame-Options, CORP, Referrer-Policy,
//!   and X-Content-Type-Options to a [`Response`].
//! - `check_post_origin` — validates Origin and Content-Type on POST requests.
//!
//! These are called inline by handlers rather than via middleware layers,
//! avoiding complex axum 0.7 type gymnastics while keeping the security
//! surface explicit and testable.

use axum::{
    http::{HeaderMap, HeaderValue, StatusCode},
    response::Response,
};

/// Configuration for POST guard checks.
#[derive(Clone, Debug)]
pub struct PostGuardConfig {
    pub allowed_origins: Vec<HeaderValue>,
    pub required_content_type: Option<HeaderValue>,
}

impl PostGuardConfig {
    /// Build a config that accepts loopback origins for the given port.
    pub fn for_loopback(port: u16) -> Self {
        let mut origins = Vec::new();
        origins.push(HeaderValue::from_str(&format!("http://127.0.0.1:{}", port)).unwrap());
        origins.push(HeaderValue::from_str(&format!("http://localhost:{}", port)).unwrap());
        Self {
            allowed_origins: origins,
            required_content_type: Some(HeaderValue::from_static(
                "application/x-www-form-urlencoded",
            )),
        }
    }
}

/// Add the five required security response headers to `response`.
pub fn add_security_headers(response: &mut Response) {
    let headers = response.headers_mut();

    headers.insert(
        "Content-Security-Policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; frame-ancestors 'none'; base-uri 'self'",
        ),
    );
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert(
        "Cross-Origin-Resource-Policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
}

/// Check POST request headers against the guard config.
///
/// Returns `Ok(())` if the request passes, or `Err(status_code, message)`
/// with the HTTP status that should be returned.
pub fn check_post_origin(
    headers: &HeaderMap,
    config: &PostGuardConfig,
) -> Result<(), (StatusCode, &'static str)> {
    // Check Origin header
    let origin_ok = match headers.get("Origin") {
        Some(origin) => config
            .allowed_origins
            .iter()
            .any(|allowed| allowed == origin),
        None => false,
    };
    if !origin_ok {
        return Err((StatusCode::FORBIDDEN, "Invalid or missing Origin header"));
    }

    // Check Content-Type header
    if let Some(required) = &config.required_content_type {
        let ct_ok = match headers.get("Content-Type") {
            Some(ct) => {
                let ct_str = ct.to_str().unwrap_or("");
                ct_str.starts_with(required.to_str().unwrap_or(""))
            }
            None => false,
        };
        if !ct_ok {
            return Err((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Unsupported Content-Type",
            ));
        }
    }

    Ok(())
}

/// Defence-in-depth check for SSE cross-origin requests.
///
/// EventSource does not send `Origin` in most browsers, so this check
/// only catches non-browser clients or future browsers that adopt the
/// behaviour. It rejects:
/// - Requests with an `Origin` header that is not loopback/same-origin.
/// - Requests with `Sec-Fetch-Site: cross-site`.
///
/// Returns `Ok(())` when the request is allowed.
pub fn check_sse_origin(headers: &HeaderMap, port: u16) -> Result<(), (StatusCode, &'static str)> {
    if let Some(origin) = headers.get("Origin") {
        let allowed: Vec<HeaderValue> = vec![
            HeaderValue::from_str(&format!("http://127.0.0.1:{}", port)).unwrap(),
            HeaderValue::from_str(&format!("http://localhost:{}", port)).unwrap(),
        ];
        if !allowed.iter().any(|a| a == origin) {
            return Err((StatusCode::FORBIDDEN, "Invalid Origin header"));
        }
    }
    if let Some(site) = headers.get("Sec-Fetch-Site") {
        if site.to_str().unwrap_or("") == "cross-site" {
            return Err((StatusCode::FORBIDDEN, "Cross-site request rejected"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Response;

    #[test]
    fn add_security_headers_sets_all_five() {
        let mut resp = Response::new(axum::body::Body::empty());
        add_security_headers(&mut resp);
        let headers = resp.headers();
        assert!(headers.get("Content-Security-Policy").is_some());
        assert!(headers.get("X-Frame-Options").is_some());
        assert!(headers.get("Cross-Origin-Resource-Policy").is_some());
        assert!(headers.get("Referrer-Policy").is_some());
        assert!(headers.get("X-Content-Type-Options").is_some());
    }

    #[test]
    fn post_guard_allows_loopback_origin() {
        let config = PostGuardConfig::for_loopback(8080);
        let mut headers = HeaderMap::new();
        headers.insert("Origin", HeaderValue::from_static("http://127.0.0.1:8080"));
        headers.insert(
            "Content-Type",
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        assert!(check_post_origin(&headers, &config).is_ok());
    }

    #[test]
    fn post_guard_rejects_foreign_origin() {
        let config = PostGuardConfig::for_loopback(8080);
        let mut headers = HeaderMap::new();
        headers.insert("Origin", HeaderValue::from_static("http://evil.com"));
        headers.insert(
            "Content-Type",
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let result = check_post_origin(&headers, &config);
        assert_eq!(result.unwrap_err().0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn post_guard_rejects_missing_origin() {
        let config = PostGuardConfig::for_loopback(8080);
        let mut headers = HeaderMap::new();
        headers.insert(
            "Content-Type",
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let result = check_post_origin(&headers, &config);
        assert_eq!(result.unwrap_err().0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn post_guard_rejects_wrong_content_type() {
        let config = PostGuardConfig::for_loopback(8080);
        let mut headers = HeaderMap::new();
        headers.insert("Origin", HeaderValue::from_static("http://127.0.0.1:8080"));
        headers.insert("Content-Type", HeaderValue::from_static("text/plain"));
        let result = check_post_origin(&headers, &config);
        assert_eq!(result.unwrap_err().0, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
}
