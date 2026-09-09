//! The cookie/token bootstrap flow: minting a terminal cookie from a query
//! token, and the port-scoping that keeps one server's cookie from being
//! honored by another.

use crate::commands::status::web;
use crate::commands::status::web::tests::{
    assert_security_headers, request, skip_without_loopback, start_with, stop, workspace,
};

use super::{connect_terminal, cookie, terminal_options, terminal_request};

#[test]
fn token_bootstrap_sets_cookie_and_redirects() {
    if skip_without_loopback("token_bootstrap_sets_cookie_and_redirects") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, _) = start_with(base, terminal_options());
    let response = request(
        port,
        &format!(
            "GET /?token={} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n",
            "a".repeat(64)
        ),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 302"));
    assert_security_headers(&response);
    for value in ["Location: /", "HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(response.contains(value), "{response}");
    }
}

#[test]
fn token_bootstrap_with_wrong_token_is_403() {
    if skip_without_loopback("token_bootstrap_with_wrong_token_is_403") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, _) = start_with(base, terminal_options());
    let response = request(
        port,
        &format!("GET /?token=wrong HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}

#[test]
fn two_servers_do_not_share_a_cookie_name() {
    if skip_without_loopback("two_servers_do_not_share_a_cookie_name") {
        return;
    }
    let (_a_temp, a_base) = workspace();
    let (_b_temp, b_base) = workspace();
    let (a_port, a_running, _) = start_with(a_base, terminal_options());
    let (b_port, b_running, _) = start_with(b_base, terminal_options());
    let token = "a".repeat(64);
    let a = request(
        a_port,
        &format!("GET /?token={token} HTTP/1.1\r\nHost: 127.0.0.1:{a_port}\r\n\r\n"),
    );
    let b = request(
        b_port,
        &format!("GET /?token={token} HTTP/1.1\r\nHost: 127.0.0.1:{b_port}\r\n\r\n"),
    );
    assert!(a.contains(&format!(
        "Set-Cookie: {}=",
        web::cookie_name_for_port(a_port)
    )));
    assert!(b.contains(&format!(
        "Set-Cookie: {}=",
        web::cookie_name_for_port(b_port)
    )));
    // The naming check above is not the property port-scoping exists for:
    // presenting B's cookie name (even carrying a token value that is valid
    // on A) must still be refused by A.
    let wrong_name = request(
        a_port,
        &terminal_request(
            a_port,
            Some(&cookie(b_port, &token)),
            Some(&format!("http://127.0.0.1:{a_port}")),
        ),
    );
    assert!(wrong_name.starts_with("HTTP/1.1 401"), "{wrong_name}");
    drop(connect_terminal(a_port, &token));
    stop(a_running);
    stop(b_running);
}
