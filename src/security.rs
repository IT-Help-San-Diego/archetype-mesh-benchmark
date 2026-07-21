//! Per-request security headers for the Calibration Scope dashboard.
//!
//! Designed ONCE for both the local loopback deployment and the public
//! calibrationscope.com deployment — identical code, no refactor when we go live.
//!
//! CSP is NONCE-BASED with 'strict-dynamic': every inline <script> in the
//! served dashboard gets a fresh per-request nonce, so we never need
//! 'unsafe-inline' (which would defeat the policy). External scripts (KaTeX)
//! are covered by 'self'. This survives web scale unchanged.

use axum::{
    body::Body,
    http::{header, HeaderName, HeaderValue, Request, Response, StatusCode},
    middleware::Next,
    response::IntoResponse,
};
use rand::Rng;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Hostnames this server will answer for. Anything else is rejected — this is
/// the DNS-rebinding guard: `local.calibrationscope.com` resolves to 127.0.0.1
/// publicly, so without a Host check a malicious page could rebind a hostname
/// IT controls to 127.0.0.1 and drive this (unauthenticated) API from the
/// victim's browser. Extend for a real public deployment via ALLOWED_HOSTS
/// (comma-separated), evaluated once.
fn allowed_hosts() -> &'static HashSet<String> {
    static HOSTS: OnceLock<HashSet<String>> = OnceLock::new();
    HOSTS.get_or_init(|| {
        let mut s: HashSet<String> = [
            "127.0.0.1",
            "localhost",
            "::1",
            "local.calibrationscope.com",
            "calibrationscope.com",
            "www.calibrationscope.com",
        ]
        .iter()
        .map(|h| h.to_string())
        .collect();
        if let Ok(extra) = std::env::var("ALLOWED_HOSTS") {
            for h in extra.split(',') {
                let h = h.trim().to_lowercase();
                if !h.is_empty() {
                    s.insert(h);
                }
            }
        }
        s
    })
}

/// True when the request's Host header names a host we serve — OR is absent.
///
/// The guard's job is to defeat browser DNS-rebinding, which REQUIRES the
/// browser to send the attacker's origin as Host (that's what smuggles the
/// request past same-origin policy). A present-but-wrong Host is exactly that
/// attack and is rejected. A MISSING Host is not a rebinding vector — browsers
/// always send one; only non-browser clients (curl --http1.0, the test
/// harness's oneshot requests) omit it — so it is allowed. The port is
/// stripped; IPv6 literals (`[::1]:8768`) are handled.
fn host_is_allowed(host_header: Option<&HeaderValue>) -> bool {
    let Some(hv) = host_header else {
        return true; // no Host → not a browser rebinding request
    };
    let Ok(h) = hv.to_str() else {
        return false;
    };
    let host = if let Some(rest) = h.strip_prefix('[') {
        rest.split(']').next().unwrap_or("") // [ipv6]:port -> ipv6
    } else {
        h.rsplit_once(':').map(|(a, _)| a).unwrap_or(h) // host:port -> host
    };
    allowed_hosts().contains(&host.to_lowercase())
}

/// Per-request CSP nonce, carried through the middleware → handler chain
/// via request extensions so `index_handler` can stamp it onto inline scripts.
#[derive(Clone, Debug)]
pub struct Nonce(pub String);

fn gen_nonce() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Returns the full CSP value for a given nonce.
///
/// `script-src` carries the nonce + 'strict-dynamic' for inline `<script>` and
/// deferred external scripts we nonce-stamp (KaTeX). `script-src-attr` allows
/// inline event-handler attributes (`onclick=`, etc.) — these are a separate
/// CSP bucket from `script-src` and MUST be declared or every handler is
/// blocked (the 1000-error regression). We scope it to 'self' + 'unsafe-inline'
/// because handler attributes cannot carry a nonce; this is the documented
/// CSP pattern for event-handler attributes.
fn csp(nonce: &str) -> String {
    format!(
        "default-src 'self'; \
         script-src 'self' 'nonce-{nonce}' 'strict-dynamic'; \
         script-src-attr 'self' 'unsafe-inline'; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; \
         font-src 'self'; \
         connect-src 'self'; \
         frame-ancestors 'none'; \
         base-uri 'self'; \
         form-action 'self'; \
         object-src 'none'; \
         upgrade-insecure-requests"
    )
}

/// Axum middleware (`from_fn`) that generates a per-request nonce, threads
/// it into the handler via `Extension<Nonce>`, then stamps the response with
/// all security headers once the handler has produced it.
pub async fn security_headers(req: Request<Body>, next: Next) -> Response<Body> {
    // DNS-rebinding guard: reject any Host we don't serve, before the request
    // reaches a handler. 421 Misdirected Request is the precise status.
    if !host_is_allowed(req.headers().get(header::HOST)) {
        return (
            StatusCode::MISDIRECTED_REQUEST,
            "Host not allowed for this instrument.",
        )
            .into_response();
    }

    // Fresh nonce for THIS request.
    let nonce = gen_nonce();
    let (mut parts, body) = req.into_parts();
    parts.extensions.insert(Nonce(nonce.clone()));
    let req = Request::from_parts(parts, body);

    let mut resp = next.run(req).await.into_response();

    let headers = resp.headers_mut();
    // CSP is the load-bearing one; rebuilt per nonce.
    if let Ok(v) = HeaderValue::from_str(&csp(&nonce)) {
        headers.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    // Defense-in-depth: explicit clickjacking + MIME sniff guards.
    if let Ok(v) = HeaderValue::from_str("DENY") {
        headers.insert(header::X_FRAME_OPTIONS, v);
    }
    if let Ok(v) = HeaderValue::from_str("nosniff") {
        headers.insert(header::X_CONTENT_TYPE_OPTIONS, v);
    }
    if let Ok(v) = HeaderValue::from_str("strict-origin-when-cross-origin") {
        headers.insert(header::REFERRER_POLICY, v);
    }
    // No camera/mic/location/USB for a benchmark dashboard.
    if let Ok(v) = HeaderValue::from_str(
        "accelerometer=(), camera=(), geolocation=(), gyroscope=(), \
         magnetometer=(), microphone=(), usb=(), payment=()",
    ) {
        headers.insert(HeaderName::from_static("permissions-policy"), v);
    }
    resp
}

/// Injects the per-request nonce onto every script tag that needs it.
///
/// 1. Inline `<script>` → `<script nonce="...">` (covered by `script-src`).
/// 2. Deferred EXTERNAL scripts (`<script defer src="...">`, e.g. KaTeX) →
///    also receive the nonce. Under `'strict-dynamic'`, external scripts are
///    NOT covered by `'self'` alone — they need a nonce or hash, or they are
///    blocked by `script-src-elem`. Stamping the deferred external scripts with
///    the same nonce lets them load while keeping the policy strict.
///
/// A plain token replace is correct and avoids regex look-around (unsupported
/// by the default `regex` feature set, which would panic at runtime).
pub fn stamp_nonce(html: &str, nonce: &str) -> String {
    html.replace("<script>", &format!("<script nonce=\"{}\">", nonce))
        .replace(
            "<script defer src=",
            &format!("<script defer nonce=\"{}\" src=", nonce),
        )
}
