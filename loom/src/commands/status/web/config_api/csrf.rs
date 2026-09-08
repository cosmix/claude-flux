//! The dashboard's double-submit CSRF token.
//!
//! # What this defends, and how
//!
//! The dashboard serves no CORS headers, so a page on another site can *send* a
//! request here but cannot *read* the response. That asymmetry is the whole
//! mechanism: the token is handed out only in the `GET /api/config` body, which
//! a cross-site page cannot read, and required in the `X-Loom-Csrf` request
//! header, which a cross-site page cannot set without a preflight that this
//! server answers with no CORS headers and so fails. Adding any
//! `Access-Control-Allow-*` header would hand both halves back to the attacker.
//!
//! It is a second layer behind the strict `Origin` check
//! (`http::origin_allowed_strict`), not a replacement for it.

use std::sync::OnceLock;

/// One token per server process, minted on first use.
///
/// Per process rather than per request: a per-request token would have to be
/// remembered somewhere to be verified, and the dashboard keeps no session
/// state. Per process still means a token that never outlives the server an
/// operator started, so a token leaked from one run is worthless against the
/// next.
static TOKEN: OnceLock<String> = OnceLock::new();

/// The token `GET /api/config` hands out.
pub(super) fn token() -> &'static str {
    TOKEN.get_or_init(mint)
}

/// 32 bytes from the OS CSPRNG, hex-encoded to 64 characters.
///
/// Two v4 UUIDs rather than a new `rand` dependency: `uuid`'s `v4` feature
/// draws from `getrandom`, i.e. the same OS entropy source, and the pair
/// carries 244 unpredictable bits — the six bits per UUID that hold the version
/// and variant are fixed, which is why this uses two rather than assuming 256.
fn mint() -> String {
    let mut bytes = [0_u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    hex::encode(bytes)
}

/// Whether `presented` is the process token, compared in constant time.
///
/// A plain `==` on strings returns at the first differing byte. A caller that
/// can time enough requests reads the token back one byte at a time from that,
/// so the fold below always visits every byte of an equal-length candidate. The
/// length itself is public — the token is a fixed 64 characters — so comparing
/// lengths first leaks nothing and keeps the fold over equal-length slices.
pub(super) fn verify(presented: Option<&str>) -> bool {
    let Some(presented) = presented else {
        return false;
    };
    let expected = token().as_bytes();
    let presented = presented.as_bytes();
    if presented.len() != expected.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (expected, presented) in expected.iter().zip(presented) {
        difference |= expected ^ presented;
    }
    difference == 0
}
