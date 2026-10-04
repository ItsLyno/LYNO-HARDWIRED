//! "Log in with Nexus": the SSO handshake that gets the player's API key
//! without them copying it from the site.
//!
//! Over a websocket to `wss://sso.nexusmods.com` the client sends a random
//! session id, opens `https://www.nexusmods.com/sso?id=<id>&application=<slug>`
//! in the browser, and the server sends the key once the player approves.
//! The application slug is issued by Nexus to registered applications only;
//! until the launcher has one, players paste a personal API key instead.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tungstenite::{Connector, Message};

use crate::{Error, Result};

pub const SSO_URL: &str = "wss://sso.nexusmods.com";

/// How long the player may take to approve in the browser.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The approval page the player opens.
pub fn browser_url(id: &str, slug: &str) -> String {
    format!("https://www.nexusmods.com/sso?id={id}&application={slug}")
}

fn sso_error(e: impl std::fmt::Display) -> Error {
    Error::Download(format!("Nexus SSO: {e}"))
}

/// `ring`, as ureq uses: no second crypto library, no process-wide default to rely on.
fn tls() -> Result<Connector> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(sso_error)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

/// `host:port` of a `ws://` / `wss://` URL.
fn address(url: &str) -> Result<String> {
    let (scheme, rest) = url.split_once("://").ok_or_else(|| sso_error(format!("bad url {url}")))?;
    let host = rest.split('/').next().unwrap_or_default();
    let port = if scheme == "wss" { 443 } else { 80 };
    Ok(if host.contains(':') { host.to_owned() } else { format!("{host}:{port}") })
}

/// Runs the handshake: `open` gets the approval page to show the player.
/// Returns the API key.
pub fn login(url: &str, slug: &str, open: &dyn Fn(&str) -> Result<()>, cancel: &dyn Fn() -> bool, timeout: Duration) -> Result<String> {
    let tcp = TcpStream::connect(address(url)?).map_err(sso_error)?;
    let (mut socket, _) = tungstenite::client_tls_with_config(url, tcp.try_clone().map_err(sso_error)?, None, Some(tls()?))
        .map_err(sso_error)?;
    // Reads wake up every second to look at `cancel`.
    tcp.set_read_timeout(Some(Duration::from_secs(1))).map_err(sso_error)?;

    let id = uuid::Uuid::new_v4().to_string();
    let hello = serde_json::json!({ "id": id, "token": null, "protocol": 2 });
    socket.send(Message::text(hello.to_string())).map_err(sso_error)?;

    let deadline = Instant::now() + timeout;
    let mut opened = false;
    loop {
        if cancel() {
            let _ = socket.close(None);
            return Err(Error::Cancelled);
        }
        if Instant::now() > deadline {
            return Err(sso_error("no answer from the browser in time"));
        }
        let text = match socket.read() {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) => return Err(sso_error("the server closed the connection")),
            Ok(_) => continue,
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(e) => return Err(sso_error(e)),
        };
        let v: serde_json::Value = serde_json::from_str(&text).map_err(sso_error)?;
        if v["success"].as_bool() == Some(false) {
            return Err(sso_error(v["error"].as_str().unwrap_or("refused")));
        }
        if let Some(key) = v["data"]["api_key"].as_str() {
            let _ = socket.close(None);
            return Ok(key.to_owned());
        }
        // The first answer confirms the session: now the page can be opened.
        if !opened && v["data"]["connection_token"].is_string() {
            open(&browser_url(&id, slug))?;
            opened = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Mutex;

    /// The SSO server's side of the protocol, over plain ws.
    fn fake_sso(approve: bool) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let hello: serde_json::Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(hello["protocol"], 2);
            assert!(hello["id"].as_str().unwrap().len() >= 32);
            ws.send(Message::text(r#"{"success":true,"data":{"connection_token":"t"},"error":null}"#)).unwrap();
            std::thread::sleep(Duration::from_millis(1500));
            let answer = match approve {
                true => r#"{"success":true,"data":{"api_key":"KEY"},"error":null}"#,
                false => r#"{"success":false,"data":null,"error":"denied"}"#,
            };
            ws.send(Message::text(answer)).unwrap();
            let _ = ws.read();
        });
        format!("ws://{addr}")
    }

    #[test]
    fn gets_the_key_after_the_page_is_opened() {
        let opened = Mutex::new(None);
        let open = |u: &str| {
            *opened.lock().unwrap() = Some(u.to_owned());
            Ok(())
        };
        let key = login(&fake_sso(true), "lyno", &open, &|| false, Duration::from_secs(10)).unwrap();
        assert_eq!(key, "KEY");
        let page = opened.lock().unwrap().clone().unwrap();
        assert!(page.starts_with("https://www.nexusmods.com/sso?id=") && page.ends_with("&application=lyno"), "{page}");
    }

    #[test]
    fn reports_a_refusal() {
        let err = login(&fake_sso(false), "lyno", &|_| Ok(()), &|| false, Duration::from_secs(10)).unwrap_err();
        assert!(err.to_string().contains("denied"), "{err}");
    }

    #[test]
    fn parses_addresses() {
        assert_eq!(address("wss://sso.nexusmods.com").unwrap(), "sso.nexusmods.com:443");
        assert_eq!(address("ws://127.0.0.1:9000/x").unwrap(), "127.0.0.1:9000");
    }
}
