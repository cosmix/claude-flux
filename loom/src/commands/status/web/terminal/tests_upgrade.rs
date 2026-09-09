//! Tests for the terminal upgrade's refusal gates and the cookie/token
//! bootstrap flow that gates them.

use std::net::TcpStream;

use crate::commands::status::web::{self, ServeOptions};
use tungstenite::client::IntoClientRequest;

mod admission;
mod bootstrap;
mod shutdown;

pub(in crate::commands::status::web) fn terminal_options() -> ServeOptions {
    ServeOptions {
        terminal_token: Some("a".repeat(64)),
    }
}

fn cookie(port: u16, value: &str) -> String {
    format!("{}={value}", web::cookie_name_for_port(port))
}

fn terminal_request(port: u16, cookie: Option<&str>, origin: Option<&str>) -> String {
    let cookie = cookie.map(|cookie| format!("Cookie: {cookie}\r\n"));
    let origin = origin.map(|origin| format!("Origin: {origin}\r\n"));
    format!(
        "GET /ws/terminal/missing/view HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{}{}\r\n",
        origin.unwrap_or_default(), cookie.unwrap_or_default()
    )
}

fn connect_terminal(port: u16, token: &str) -> tungstenite::WebSocket<TcpStream> {
    let url = format!("ws://127.0.0.1:{port}/ws/terminal/missing/view");
    let mut request = url
        .as_str()
        .into_client_request()
        .expect("terminal request");
    request.headers_mut().insert(
        "Origin",
        format!("http://127.0.0.1:{port}").parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("Cookie", cookie(port, token).parse().unwrap());
    tungstenite::client(request, TcpStream::connect(("127.0.0.1", port)).unwrap())
        .expect("terminal handshake")
        .0
}
