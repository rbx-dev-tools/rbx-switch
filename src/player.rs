//! Switching the account the Roblox Player is signed in as.
//!
//! Studio and the Player keep separate sessions, and the Player has no file
//! this tool could rewrite the way `RobloxStudioAuthuserid` is rewritten for
//! Studio. What it does accept is the launch the website sends when somebody
//! presses Play: a `roblox-player:` URI carrying `gameinfo:<ticket>`, a
//! single-use authentication ticket issued for one account. The Player redeems
//! the ticket and, measured on Windows on 2026-11-08, stays signed in as that
//! account afterwards. So a ticket is a switch, not a one-off join.
//!
//! The ticket comes from `auth.roblox.com`, requested with the account's own
//! Studio session. That makes this the one path in the tool that reads a
//! session and makes a network call; the session goes to `auth.roblox.com`
//! and nowhere else, and is never printed.
//!
//! `launchmode:app` opens the Player on its home screen rather than joining a
//! place: switching is this tool's job, and joining is left to whatever runs
//! next (`rbx open --play`, the website, the Player itself).

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

const TICKET_URL: &str = "https://auth.roblox.com/v1/authentication-ticket";

/// The value of an account's `.ROBLOSECURITY` cookie.
///
/// A newtype with a redacting `Debug` so that no `{:?}`, error chain or panic
/// message can print it by accident.
pub struct Session(String);

impl Session {
    /// Accepts a stored value only if it has the shape of the cookie. Every
    /// `.ROBLOSECURITY` starts with Roblox's `_|WARNING` banner; anything else
    /// means Studio changed how it stores the session, and sending it would
    /// only earn a 401 that blames the account.
    pub fn parse(raw: &str) -> Option<Self> {
        let value = raw.trim();
        value
            .starts_with("_|WARNING")
            .then(|| Self(value.to_string()))
    }

    fn cookie_header(&self) -> String {
        format!(".ROBLOSECURITY={}", self.0)
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Session(<redacted>)")
    }
}

/// What one POST to the ticket endpoint answered, reduced to the three
/// things the exchange looks at.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TicketResponse {
    pub status: u16,
    pub csrf_token: Option<String>,
    pub ticket: Option<String>,
}

/// One POST to the ticket endpoint. A trait so the CSRF exchange can be
/// tested without a network.
pub trait TicketTransport {
    fn post(&self, session: &Session, csrf_token: Option<&str>) -> Result<TicketResponse>;
}

/// Asks Roblox for a ticket, following the CSRF exchange every
/// state-changing `roblox.com` endpoint uses: the first POST is refused with
/// 403 and an `x-csrf-token` header, and the same POST carrying that token is
/// the one that succeeds.
///
/// `who` names the account in errors, so a rejected session says whose it is.
pub fn request_ticket(
    transport: &dyn TicketTransport,
    session: &Session,
    who: &str,
) -> Result<String> {
    let mut response = transport.post(session, None)?;

    if response.status == 403 {
        let Some(token) = response.csrf_token.take() else {
            bail!("Roblox refused the sign-in ticket request (HTTP 403) without a CSRF token");
        };
        response = transport.post(session, Some(&token))?;
    }

    match response.status {
        200 => response.ticket.filter(|t| !t.is_empty()).with_context(|| {
            "Roblox answered the ticket request without a ticket. \
             The endpoint may have changed."
        }),
        401 => bail!(
            "Roblox rejected the Studio session of {who} (HTTP 401): it has expired \
             or was signed out. Sign in to Studio as {who} again, then retry."
        ),
        status => bail!("Roblox did not issue a sign-in ticket for {who} (HTTP {status})"),
    }
}

/// The real transport. Blocking, because the whole command is two requests
/// in a row and nothing else.
#[derive(Debug)]
pub struct HttpTransport {
    agent: ureq::Agent,
}

impl HttpTransport {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            // The 403 carries the CSRF token: it is a step, not a failure.
            .http_status_as_error(false)
            .user_agent(concat!("rbx-account/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent }
    }
}

impl TicketTransport for HttpTransport {
    fn post(&self, session: &Session, csrf_token: Option<&str>) -> Result<TicketResponse> {
        let mut request = self
            .agent
            .post(TICKET_URL)
            .header("Cookie", session.cookie_header())
            // The endpoint refuses a request whose Referer is not roblox.com.
            .header("Referer", "https://www.roblox.com/")
            // And one with no Content-Type, with 415, even though the body is
            // empty. This is the type a browser form POST carries.
            .header("Content-Type", "application/x-www-form-urlencoded");
        if let Some(token) = csrf_token {
            request = request.header("x-csrf-token", token);
        }

        let response = request
            .send("")
            .context("Could not reach auth.roblox.com")?;

        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        };

        Ok(TicketResponse {
            status: response.status().as_u16(),
            csrf_token: header("x-csrf-token"),
            ticket: header("rbx-authentication-ticket"),
        })
    }
}

/// The URI that signs the Player in with `ticket` and opens it on its home
/// screen.
///
/// The fields are the website's launch minus everything about a place.
/// `launchtime` is the moment of the launch in milliseconds and
/// `browsertrackerid` an id the website would take from its tracking cookie;
/// any number is accepted, so the launch time is reused.
pub fn launch_uri(ticket: &str, launch_ms: u128) -> String {
    format!(
        "roblox-player:1+launchmode:app+gameinfo:{ticket}+launchtime:{launch_ms}\
         +browsertrackerid:{launch_ms}+channel:"
    )
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Plays back canned responses and records the CSRF token of each call.
    struct Scripted {
        responses: RefCell<Vec<TicketResponse>>,
        tokens_sent: RefCell<Vec<Option<String>>>,
    }

    impl Scripted {
        fn new(responses: Vec<TicketResponse>) -> Self {
            Self {
                responses: RefCell::new(responses),
                tokens_sent: RefCell::new(Vec::new()),
            }
        }
    }

    impl TicketTransport for Scripted {
        fn post(&self, _: &Session, csrf_token: Option<&str>) -> Result<TicketResponse> {
            self.tokens_sent
                .borrow_mut()
                .push(csrf_token.map(String::from));
            Ok(self.responses.borrow_mut().remove(0))
        }
    }

    fn session() -> Session {
        Session::parse("_|WARNING:-DO-NOT-SHARE-THIS.--aaaaBBBBccccDDDD").unwrap()
    }

    fn csrf_challenge() -> TicketResponse {
        TicketResponse {
            status: 403,
            csrf_token: Some("token123".into()),
            ticket: None,
        }
    }

    #[test]
    fn the_csrf_challenge_is_answered_with_its_token() {
        let transport = Scripted::new(vec![
            csrf_challenge(),
            TicketResponse {
                status: 200,
                csrf_token: None,
                ticket: Some("ticket-abc".into()),
            },
        ]);

        let ticket = request_ticket(&transport, &session(), "QaAccount").unwrap();

        assert_eq!(ticket, "ticket-abc");
        assert_eq!(
            *transport.tokens_sent.borrow(),
            vec![None, Some("token123".to_string())],
            "the retry has to carry the token the 403 handed out"
        );
    }

    #[test]
    fn a_rejected_session_names_the_account_and_the_fix() {
        let transport = Scripted::new(vec![
            csrf_challenge(),
            TicketResponse {
                status: 401,
                ..Default::default()
            },
        ]);

        let error = request_ticket(&transport, &session(), "QaAccount").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("QaAccount"), "{message}");
        assert!(message.contains("Sign in to Studio"), "{message}");
    }

    #[test]
    fn a_403_without_a_token_is_not_retried() {
        let transport = Scripted::new(vec![TicketResponse {
            status: 403,
            ..Default::default()
        }]);

        let error = request_ticket(&transport, &session(), "QaAccount").unwrap_err();
        assert!(error.to_string().contains("CSRF"), "{error}");
        assert_eq!(transport.tokens_sent.borrow().len(), 1);
    }

    #[test]
    fn a_200_without_a_ticket_is_an_error_not_an_empty_launch() {
        let transport = Scripted::new(vec![TicketResponse {
            status: 200,
            ticket: Some(String::new()),
            ..Default::default()
        }]);

        assert!(request_ticket(&transport, &session(), "QaAccount").is_err());
    }

    #[test]
    fn a_stored_value_without_the_banner_is_refused() {
        assert!(Session::parse("COOK::<something>").is_none());
        assert!(Session::parse("").is_none());
    }

    #[test]
    fn a_session_never_prints() {
        let printed = format!("{:?}", session());
        assert!(!printed.contains("WARNING"), "{printed}");
    }

    #[test]
    fn the_uri_opens_the_app_and_names_no_place() {
        let uri = launch_uri("ticket-abc", 1_700_000_000_000);
        assert!(uri.starts_with("roblox-player:1+launchmode:app+"), "{uri}");
        assert!(uri.contains("+gameinfo:ticket-abc+"), "{uri}");
        assert!(!uri.contains("placelauncherurl"), "{uri}");
    }
}
