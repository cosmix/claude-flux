//! Per-dashboard terminal token helpers.

use std::fs::File;
use std::io::{Read, Result};

/// 32 bytes from `/dev/urandom` as 64 hex chars (same shape as the daemon's tokens).
pub(super) fn mint() -> Result<String> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(hex::encode(bytes))
}

/// Constant-time equality over bytes; `None` never matches.
pub(super) fn matches(presented: Option<&str>, expected: &str) -> bool {
    let Some(presented) = presented else {
        return false;
    };
    let difference = presented
        .bytes()
        .zip(expected.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        });
    presented.len() == expected.len() && difference == 0
}

/// The cookie name for a dashboard bound to `port`.
pub(super) fn cookie_name(port: u16) -> String {
    format!("loom_dashboard_{port}")
}

/// The value of cookie `name` out of a raw `Cookie` header.
pub(super) fn cookie_token<'a>(cookie_header: Option<&'a str>, name: &str) -> Option<&'a str> {
    cookie_header?.split(';').find_map(|entry| {
        let (key, value) = entry.trim().split_once('=')?;
        (key == name).then_some(value)
    })
}

/// The `token` value out of a raw query string; no percent-decoding (hex only).
pub(super) fn query_token(query: Option<&str>) -> Option<&str> {
    query?.split('&').find_map(|entry| {
        let (key, value) = entry.split_once('=')?;
        (key == "token").then_some(value)
    })
}
