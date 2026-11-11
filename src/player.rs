//! Switching the account the Roblox Player is signed in as.
//!
//! Studio and the Player keep separate sessions. Both paths below start from a
//! single-use authentication ticket, requested from `auth.roblox.com` with the
//! account's own Studio session: the ticket the website gets when somebody
//! presses Play.
//!
//! The ticket is redeemed here, for a session of the Player's own, and that
//! session replaces `.ROBLOSECURITY` in the Player's cookie store. Nothing is
//! launched: the Player opens signed in as the account next time it starts,
//! the way a Studio switch only rewrites `RobloxStudioAuthuserid`. Measured on
//! Windows on 2026-11-09: swapping the session alone, every other cookie kept,
//! is enough. An open Player rewrites its store when it exits, so it has to be
//! closed first (`--force` closes it).
//!
//! This is the one part of the tool that reads a session and makes network
//! calls. Sessions go to Roblox's own hosts over HTTPS, never through a
//! redirect, and are never printed.

use std::fmt;
use std::time::Duration;

use anyhow::{bail, Context, Result};

const TICKET_URL: &str = "https://auth.roblox.com/v1/authentication-ticket";
const REDEEM_URL: &str = "https://auth.roblox.com/v1/authentication-ticket/redeem";
const AUTHENTICATED_URL: &str = "https://users.roblox.com/v1/users/authenticated";

/// The cookie the session lives in, in the Player's store as in a browser.
const SESSION_COOKIE: &str = ".ROBLOSECURITY";

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
        format!("{SESSION_COOKIE}={}", self.0)
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

/// How long a connection may take to open, TLS handshake included.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one read may wait for a byte, headers and body alike.
const READ_TIMEOUT: Duration = Duration::from_secs(20);
/// The most one request may take from start to finish.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

impl HttpTransport {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            // The 403 carries the CSRF token: it is a step, not a failure.
            .http_status_as_error(false)
            // The session goes to auth.roblox.com and nowhere else: no
            // redirect is followed, and nothing is sent over plain HTTP.
            .max_redirects(0)
            .https_only(true)
            // Without these, a host that accepts the connection and never
            // answers hangs the switch for good, with no message.
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_per_read(Some(READ_TIMEOUT))
            .timeout_per_call(Some(CALL_TIMEOUT))
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

impl RedeemTransport for HttpTransport {
    fn redeem(&self, ticket: &str, csrf_token: Option<&str>) -> Result<RedeemResponse> {
        let mut request = self
            .agent
            .post(REDEEM_URL)
            .header("Referer", "https://www.roblox.com/")
            // Without it the endpoint refuses the ticket: this is how the
            // Player announces a ticket sign-in.
            .header("RBXAuthenticationNegotiation", "1")
            .header("Content-Type", "application/json");
        if let Some(token) = csrf_token {
            request = request.header("x-csrf-token", token);
        }

        let body = serde_json::json!({ "authenticationTicket": ticket }).to_string();
        let response = request
            .send(body.as_str())
            .context("Could not reach auth.roblox.com")?;

        let session = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(session_from_set_cookie);

        Ok(RedeemResponse {
            status: response.status().as_u16(),
            csrf_token: response
                .headers()
                .get("x-csrf-token")
                .and_then(|v| v.to_str().ok())
                .map(String::from),
            session,
        })
    }
}

impl HttpTransport {
    /// The user id a session belongs to. Checked before a session is written
    /// anywhere, so a ticket that came back for somebody else cannot sign the
    /// Player in as them.
    pub fn user_id(&self, session: &Session) -> Result<String> {
        let mut response = self
            .agent
            .get(AUTHENTICATED_URL)
            .header("Cookie", session.cookie_header())
            .call()
            .context("Could not reach users.roblox.com")?;
        let status = response.status().as_u16();
        if status != 200 {
            bail!("Roblox did not say whose the new session is (HTTP {status})");
        }

        #[derive(serde::Deserialize)]
        struct Me {
            id: u64,
        }
        let body = response
            .body_mut()
            .read_to_string()
            .context("Unexpected answer from users.roblox.com")?;
        let me: Me =
            serde_json::from_str(&body).context("Unexpected answer from users.roblox.com")?;
        Ok(me.id.to_string())
    }
}

/// The session in one `Set-Cookie` header, if that header sets it.
fn session_from_set_cookie(header: &str) -> Option<Session> {
    let value = header.strip_prefix(SESSION_COOKIE)?.strip_prefix('=')?;
    Session::parse(value.split(';').next()?)
}

/// What one POST to the redeem endpoint answered.
#[derive(Debug, Default)]
pub struct RedeemResponse {
    pub status: u16,
    pub csrf_token: Option<String>,
    pub session: Option<Session>,
}

/// One POST to the redeem endpoint. A trait for the same reason as
/// [`TicketTransport`].
pub trait RedeemTransport {
    fn redeem(&self, ticket: &str, csrf_token: Option<&str>) -> Result<RedeemResponse>;
}

/// Trades a ticket for a new session, as the Player does when it is launched
/// with one. The session is the Player's own: Studio's is not shared, so
/// signing out of one app does not sign out the other.
///
/// The endpoint is answered with the same CSRF exchange as the ticket request.
pub fn redeem_ticket(transport: &dyn RedeemTransport, ticket: &str) -> Result<Session> {
    let mut response = transport.redeem(ticket, None)?;

    if response.status == 403 {
        if let Some(token) = response.csrf_token.take() {
            response = transport.redeem(ticket, Some(&token))?;
        }
    }

    match response.status {
        200 => response.session.with_context(|| {
            "Roblox redeemed the sign-in ticket without setting a session. \
             The endpoint may have changed."
        }),
        status => bail!("Roblox did not redeem the sign-in ticket (HTTP {status})"),
    }
}

/// Puts `session` in place of the one in the Player's cookie store.
///
/// `store` is the store decrypted: Netscape cookie lines (domain, subdomains,
/// path, secure, expiry, name, value, separated by tabs) joined by `"; "`.
/// Only the value of `.ROBLOSECURITY` changes; every other cookie, and the
/// session's own expiry, stays as it was.
pub fn replace_session(store: &str, session: &Session) -> Result<String> {
    let mut found = false;
    let cookies: Vec<String> = store
        .split("; ")
        .map(|cookie| {
            let mut fields: Vec<&str> = cookie.split('\t').collect();
            if is_session_cookie(&fields) {
                found = true;
                fields[6] = &session.0;
                fields.join("\t")
            } else {
                cookie.to_string()
            }
        })
        .collect();

    if !found {
        bail!("the Player is signed out, so it holds no session to replace");
    }
    Ok(cookies.join("; "))
}

/// Whether [`replace_session`] would find a session to replace in `store`.
/// Asked before anything changes, so a signed-out Player is refused before a
/// ticket is spent on it.
pub fn holds_session(store: &str) -> bool {
    store
        .split("; ")
        .any(|cookie| is_session_cookie(&cookie.split('\t').collect::<Vec<_>>()))
}

/// A cookie line's fields are the session's: seven of them, the sixth its name.
fn is_session_cookie(fields: &[&str]) -> bool {
    fields.len() == 7 && fields[5] == SESSION_COOKIE
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
    fn a_200_without_a_ticket_is_an_error_not_an_empty_ticket() {
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

    /// A store shaped like the Player's: the session among cookies that must
    /// survive the swap untouched, one of them holding spaces.
    const STORE: &str = "#HttpOnly_.roblox.com\tTRUE\t/\tTRUE\t0\t.RBXIDCHECK\tid-check; \
        .roblox.com\tTRUE\t/\tFALSE\t1828783200\tRBXEventTrackerV2\tCreateDate=11/9/2026 05:19:55&rbxid=1234567890; \
        #HttpOnly_.roblox.com\tTRUE\t/\tTRUE\t1828783200\t.ROBLOSECURITY\t_|WARNING:-old-session; \
        #HttpOnly_.roblox.com\tTRUE\t/\tTRUE\t1828783200\trbxas\tabcdef";

    #[test]
    fn only_the_session_value_changes() {
        let replaced = replace_session(STORE, &session()).unwrap();

        assert_eq!(
            replaced,
            STORE.replace(
                "_|WARNING:-old-session",
                "_|WARNING:-DO-NOT-SHARE-THIS.--aaaaBBBBccccDDDD"
            )
        );
        assert!(
            replaced.contains("\t1828783200\t.ROBLOSECURITY\t"),
            "the expiry is kept"
        );
    }

    #[test]
    fn a_store_without_a_session_is_refused_rather_than_guessed_at() {
        let signed_out = "#HttpOnly_.roblox.com\tTRUE\t/\tFALSE\t0\tGuestData\tUserID=-1";
        let error = replace_session(signed_out, &session()).unwrap_err();
        assert!(error.to_string().contains("signed out"), "{error}");
        assert!(!holds_session(signed_out));
    }

    #[test]
    fn a_signed_in_store_holds_a_session() {
        assert!(holds_session(STORE));
        let named_only = "x.com\tTRUE\t/\tFALSE\t0\tnote\t.ROBLOSECURITY";
        assert!(!holds_session(named_only), "a value is not a name");
    }

    #[test]
    fn a_cookie_whose_value_mentions_the_session_name_is_left_alone() {
        let store = "x.com\tTRUE\t/\tFALSE\t0\tnote\t.ROBLOSECURITY; \
            #HttpOnly_.roblox.com\tTRUE\t/\tTRUE\t0\t.ROBLOSECURITY\t_|WARNING:-old";
        let replaced = replace_session(store, &session()).unwrap();
        assert!(
            replaced.starts_with("x.com\tTRUE\t/\tFALSE\t0\tnote\t.ROBLOSECURITY; "),
            "{replaced}"
        );
    }

    #[test]
    fn the_session_is_read_from_its_own_set_cookie_only() {
        let set = ".ROBLOSECURITY=_|WARNING:-new; domain=.roblox.com; path=/; secure; HttpOnly";
        assert_eq!(session_from_set_cookie(set).unwrap().0, "_|WARNING:-new");
        assert!(session_from_set_cookie("RBXSessionTracker=abc; path=/").is_none());
        assert!(session_from_set_cookie(".ROBLOSECURITYX=_|WARNING:-x").is_none());
        // Roblox clears the cookie this way on sign-out: not a session.
        assert!(session_from_set_cookie(".ROBLOSECURITY=; expires=Thu, 01 Jan 1970").is_none());
    }

    /// Built without sending anything: the agent connects only on a request.
    #[test]
    fn every_request_gives_up_instead_of_hanging() {
        let transport = HttpTransport::new();
        let config = transport.agent.config();
        let timeouts = config.timeouts();
        assert_eq!(timeouts.connect, Some(CONNECT_TIMEOUT));
        assert_eq!(config.timeout_per_read(), Some(READ_TIMEOUT));
        assert_eq!(timeouts.per_call, Some(CALL_TIMEOUT));
    }

    struct Redeems(RefCell<Vec<RedeemResponse>>);

    impl RedeemTransport for Redeems {
        fn redeem(&self, _: &str, _: Option<&str>) -> Result<RedeemResponse> {
            Ok(self.0.borrow_mut().remove(0))
        }
    }

    #[test]
    fn a_redeem_answers_its_csrf_challenge_too() {
        let transport = Redeems(RefCell::new(vec![
            RedeemResponse {
                status: 403,
                csrf_token: Some("token123".into()),
                session: None,
            },
            RedeemResponse {
                status: 200,
                csrf_token: None,
                session: Some(session()),
            },
        ]));
        assert!(redeem_ticket(&transport, "ticket-abc").is_ok());
    }

    #[test]
    fn a_redeem_without_a_session_is_an_error() {
        let transport = Redeems(RefCell::new(vec![RedeemResponse {
            status: 200,
            ..Default::default()
        }]));
        assert!(redeem_ticket(&transport, "ticket-abc").is_err());
    }
}
