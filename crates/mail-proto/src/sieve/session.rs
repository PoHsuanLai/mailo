//! A ManageSieve client (RFC 5804), as a [`Machine`].
//!
//! One connection does one job — look, install, or remove — because each is a short walk and
//! the decision in the middle of it (is someone else's script active?) needs the listing the
//! walk itself fetches. The caller never sees a half-installed state: the machine lists first,
//! refuses before writing anything, and only then puts and activates.
//!
//! **TLS is not optional.** ManageSieve has no implicit-TLS port; its norm is `STARTTLS` on
//! 4190. With [`Tls::StartTlsRequired`] the upgrade is mandatory: a server that does not offer
//! `STARTTLS`, or refuses it, ends the session before a credential is written, and bytes that
//! arrive after the `OK` to `STARTTLS` but before the handshake are refused rather than read as
//! the TLS session's capabilities — that is the shape of a command-injection attack. The
//! capabilities used afterwards are the ones re-issued inside TLS (RFC 5804 §2.2), never the
//! plaintext ones.

use super::SCRIPT_NAME;
use super::wire::{self, Token};
use crate::machine::{IoNeed, IoReady, Machine, Progress, ProtoError, Refusal};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use mail_domain::{Credential, Tls};

/// Where to connect and who to be.
///
/// `Debug` is derived: [`Credential`]'s own `Debug` redacts the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SieveLogin {
    /// Server name, for [`IoNeed::OpenTls`].
    pub host: String,
    pub port: u16,
    pub tls: Tls,
    pub username: String,
    pub credential: Credential,
}

/// What the connection is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SieveJob {
    /// List the scripts, and fetch this client's if it has one.
    Status,
    /// Put this script up as this client's and make it the active one.
    Install { script: String, takeover: Takeover },
    /// Take this client's script down: deactivate it if it is active, then delete it.
    Remove,
}

/// What to do when a script this client did not write is the active one.
///
/// A server runs one script at a time, so activating ours switches theirs off — and theirs may
/// be every rule the user wrote in webmail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Takeover {
    /// Stop and say whose it is. The default.
    Refuse,
    /// Activate ours anyway. Theirs is left on the server, inactive, not deleted.
    Replace,
}

/// What the server said it can do, from its capability response.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SieveCaps {
    pub implementation: Option<String>,
    /// The Sieve extensions it runs, as tokens: `fileinto`, `vacation`, `date`.
    pub sieve: Vec<String>,
    /// SASL mechanisms it accepts, upper case.
    pub sasl: Vec<String>,
    pub starttls: Offered,
}

/// Whether the server offered an upgrade to TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Offered {
    Yes,
    #[default]
    No,
}

impl SieveCaps {
    /// Whether the server's Sieve has `extension`, compared as a whole token.
    pub fn has_extension(&self, extension: &str) -> bool {
        self.sieve.iter().any(|e| e.eq_ignore_ascii_case(extension))
    }

    fn read(lines: &[Vec<Token>]) -> SieveCaps {
        let mut caps = SieveCaps::default();
        for line in lines {
            let Some(key) = line.first().map(Token::text) else {
                continue;
            };
            let value = line.get(1).map(Token::text).unwrap_or_default();
            match key.to_ascii_uppercase().as_str() {
                "IMPLEMENTATION" => caps.implementation = Some(value),
                "SIEVE" => {
                    caps.sieve = value
                        .split_ascii_whitespace()
                        .map(str::to_ascii_lowercase)
                        .collect();
                }
                "SASL" => {
                    caps.sasl = value
                        .split_ascii_whitespace()
                        .map(str::to_ascii_uppercase)
                        .collect();
                }
                "STARTTLS" => caps.starttls = Offered::Yes,
                _ => {}
            }
        }
        caps
    }
}

/// One script the server holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptEntry {
    pub name: String,
    pub active: Active,
}

/// Whether a script is the one the server runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Active {
    Yes,
    No,
}

impl ScriptEntry {
    fn ours(&self) -> bool {
        self.name == SCRIPT_NAME
    }
}

/// What a removal found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deleted {
    /// This client's script was there and is gone.
    Ours,
    /// There was nothing of this client's to remove.
    NothingThere,
}

/// How the job ended. Every variant carries the capabilities, which is what `status` reports
/// and what a caller compiles the next script against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SieveOutcome {
    Status {
        caps: SieveCaps,
        scripts: Vec<ScriptEntry>,
        /// This client's script as the server holds it.
        ours: Option<String>,
    },
    Installed {
        caps: SieveCaps,
        /// The script that was active before, when it was someone else's and
        /// [`Takeover::Replace`] switched it off.
        displaced: Option<String>,
    },
    /// Nothing was written: another script is active and [`Takeover::Refuse`] stood.
    Refused {
        caps: SieveCaps,
        active: String,
    },
    Removed {
        caps: SieveCaps,
        deleted: Deleted,
    },
}

/// Where the dialogue is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// The capability lines the server opens with, before `OK`.
    Greeting,
    StartTls,
    /// Waiting for the handshake the runtime performs.
    AwaitTls,
    /// The capabilities re-issued inside TLS.
    Reissued,
    Authenticate,
    /// A SASL challenge was answered with an empty response; the final answer follows.
    AuthCancelled,
    List,
    Get,
    Put,
    Activate,
    Deactivate,
    Delete,
    Logout,
    Finished,
}

/// A ManageSieve session doing one [`SieveJob`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SieveSession {
    login: SieveLogin,
    job: SieveJob,
    phase: Phase,
    buf: Vec<u8>,
    /// Data lines of the response in flight.
    lines: Vec<Vec<Token>>,
    caps: SieveCaps,
    scripts: Vec<ScriptEntry>,
    ours: Option<String>,
    /// Decided before `LOGOUT`, handed back when the server has answered it.
    outcome: Option<SieveOutcome>,
}

/// What handling one complete response line did.
enum Step {
    /// Read the next line.
    More,
    Need(Vec<IoNeed>),
    Done(SieveOutcome),
    Fail(ProtoError),
}

impl SieveSession {
    pub fn new(login: SieveLogin, job: SieveJob) -> Self {
        Self {
            login,
            job,
            phase: Phase::Greeting,
            buf: Vec::new(),
            lines: Vec::new(),
            caps: SieveCaps::default(),
            scripts: Vec::new(),
            ours: None,
            outcome: None,
        }
    }

    fn pump(&mut self) -> Progress<SieveOutcome> {
        loop {
            let (tokens, used) = match wire::line(&self.buf) {
                Ok(Some(found)) => found,
                Ok(None) => return Progress::Need(vec![IoNeed::Read]),
                Err(e) => return self.fail(e),
            };
            self.buf.drain(..used);
            match self.handle(tokens) {
                Step::More => {}
                Step::Need(mut needs) => {
                    // A write is always followed by reading its answer, except the TLS
                    // handshake, which the runtime answers with `TlsOpen`.
                    if !needs.iter().any(|n| matches!(n, IoNeed::OpenTls { .. })) {
                        needs.push(IoNeed::Read);
                    }
                    return Progress::Need(needs);
                }
                Step::Done(out) => {
                    self.phase = Phase::Finished;
                    return Progress::Done(out);
                }
                Step::Fail(e) => return self.fail(e),
            }
        }
    }

    fn fail(&mut self, e: ProtoError) -> Progress<SieveOutcome> {
        self.phase = Phase::Finished;
        Progress::Failed(e)
    }

    /// One line: a data line is kept for the response in flight; `OK`, `NO` or `BYE` ends it.
    fn handle(&mut self, tokens: Vec<Token>) -> Step {
        let status = match tokens.first() {
            Some(Token::Atom(word)) => match word.to_ascii_uppercase().as_str() {
                "OK" => Status::Ok,
                "NO" => Status::No,
                "BYE" => Status::Bye,
                _ => Status::Data,
            },
            _ => Status::Data,
        };
        if status == Status::Data {
            return self.data(tokens);
        }
        let text = tokens
            .iter()
            .skip(1)
            .find(|t| matches!(t, Token::Text(_)))
            .map(Token::text)
            .unwrap_or_default();
        let lines = std::mem::take(&mut self.lines);
        self.finished(status, text, lines)
    }

    fn data(&mut self, tokens: Vec<Token>) -> Step {
        if self.phase == Phase::Authenticate {
            // A SASL challenge. PLAIN and XOAUTH2 both send everything up front, so a challenge
            // is XOAUTH2's error report; an empty response asks for the final `NO`, and sending
            // the token again would put it on the wire twice.
            self.phase = Phase::AuthCancelled;
            return Step::Need(vec![IoNeed::Write(b"\"\"\r\n".to_vec())]);
        }
        self.lines.push(tokens);
        Step::More
    }

    fn finished(&mut self, status: Status, text: String, lines: Vec<Vec<Token>>) -> Step {
        match (&self.phase, status) {
            (Phase::Greeting | Phase::Reissued, Status::Ok) => {
                self.caps = SieveCaps::read(&lines);
                self.after_capabilities()
            }
            (Phase::StartTls, Status::Ok) => {
                // Anything already here arrived in plaintext after the server agreed to
                // upgrade: it cannot be the TLS session's, and reading it as such is how a
                // man in the middle injects capabilities.
                if !self.buf.is_empty() {
                    return Step::Fail(ProtoError::Malformed(
                        "bytes after STARTTLS, before the handshake".to_owned(),
                    ));
                }
                self.phase = Phase::AwaitTls;
                Step::Need(vec![IoNeed::OpenTls {
                    host: self.login.host.clone(),
                    port: self.login.port,
                    mode: self.login.tls,
                }])
            }
            (Phase::Authenticate | Phase::AuthCancelled, Status::Ok) => self.list(),
            (Phase::Authenticate | Phase::AuthCancelled, _) => {
                Step::Fail(ProtoError::AuthRejected(text))
            }
            (Phase::List, Status::Ok) => {
                self.scripts = lines.iter().filter_map(|l| entry(l)).collect();
                self.after_list()
            }
            (Phase::Get, Status::Ok) => {
                self.ours = lines.first().and_then(|l| l.first()).map(Token::text);
                self.logout(SieveOutcome::Status {
                    caps: self.caps.clone(),
                    scripts: self.scripts.clone(),
                    ours: self.ours.clone(),
                })
            }
            (Phase::Put, Status::Ok) => self.activate(),
            (Phase::Activate, Status::Ok) => {
                let displaced = self
                    .scripts
                    .iter()
                    .find(|s| s.active == Active::Yes && !s.ours())
                    .map(|s| s.name.clone());
                self.logout(SieveOutcome::Installed {
                    caps: self.caps.clone(),
                    displaced,
                })
            }
            (Phase::Deactivate, Status::Ok) => self.delete(),
            (Phase::Delete, Status::Ok) => self.logout(SieveOutcome::Removed {
                caps: self.caps.clone(),
                deleted: Deleted::Ours,
            }),
            (Phase::Logout, _) => match self.outcome.take() {
                Some(out) => Step::Done(out),
                None => Step::Fail(ProtoError::Malformed("LOGOUT before a result".to_owned())),
            },
            // A server that says goodbye unasked, or refuses a step: its words are the reason.
            (_, Status::Bye) => Step::Fail(ProtoError::Refused {
                kind: Refusal::Transient,
                text,
            }),
            (Phase::StartTls, _) => Step::Fail(ProtoError::Unsupported(format!(
                "STARTTLS was refused: {text}"
            ))),
            (Phase::Put, _) => Step::Fail(ProtoError::Refused {
                kind: Refusal::Permanent,
                text: format!("the server rejected the script: {text}"),
            }),
            (_, _) => Step::Fail(ProtoError::Refused {
                kind: Refusal::Permanent,
                text,
            }),
        }
    }

    /// Upgrade first when the upgrade is required and has not happened; then sign in.
    fn after_capabilities(&mut self) -> Step {
        if self.login.tls == Tls::StartTlsRequired && self.phase == Phase::Greeting {
            if self.caps.starttls != Offered::Yes {
                return Step::Fail(ProtoError::Unsupported(
                    "STARTTLS: the server does not offer it, and signing in without it would \
                     send the password in the clear"
                        .to_owned(),
                ));
            }
            self.phase = Phase::StartTls;
            return Step::Need(vec![IoNeed::Write(b"STARTTLS\r\n".to_vec())]);
        }
        self.authenticate()
    }

    fn authenticate(&mut self) -> Step {
        let (mechanism, raw) = match &self.login.credential {
            Credential::Password(password) => ("PLAIN", plain(&self.login.username, password)),
            Credential::OAuth { access, .. } => ("XOAUTH2", xoauth2(&self.login.username, access)),
            Credential::OpenPgp(_) => {
                return Step::Fail(ProtoError::Unsupported(
                    "an OpenPGP key is not a sign-in credential".to_owned(),
                ));
            }
        };
        if !self.caps.sasl.iter().any(|m| m == mechanism) {
            return Step::Fail(ProtoError::Unsupported(format!(
                "AUTHENTICATE {mechanism}: the server offers {}",
                self.caps.sasl.join(" ")
            )));
        }
        self.phase = Phase::Authenticate;
        let line = format!(
            "AUTHENTICATE \"{mechanism}\" \"{}\"\r\n",
            STANDARD.encode(raw)
        );
        Step::Need(vec![IoNeed::Write(line.into_bytes())])
    }

    fn list(&mut self) -> Step {
        self.phase = Phase::List;
        Step::Need(vec![IoNeed::Write(b"LISTSCRIPTS\r\n".to_vec())])
    }

    fn after_list(&mut self) -> Step {
        let ours = self.scripts.iter().find(|s| s.ours()).cloned();
        match self.job.clone() {
            SieveJob::Status => match ours {
                Some(_) => {
                    self.phase = Phase::Get;
                    let line = format!("GETSCRIPT {}\r\n", string(SCRIPT_NAME));
                    Step::Need(vec![IoNeed::Write(line.into_bytes())])
                }
                None => self.logout(SieveOutcome::Status {
                    caps: self.caps.clone(),
                    scripts: self.scripts.clone(),
                    ours: None,
                }),
            },
            SieveJob::Install { script, takeover } => {
                let theirs = self
                    .scripts
                    .iter()
                    .find(|s| s.active == Active::Yes && !s.ours());
                if let (Some(theirs), Takeover::Refuse) = (theirs, takeover) {
                    let active = theirs.name.clone();
                    return self.logout(SieveOutcome::Refused {
                        caps: self.caps.clone(),
                        active,
                    });
                }
                self.phase = Phase::Put;
                // `{n+}`: a client literal is always non-synchronizing in ManageSieve
                // (RFC 5804 §4), so the script follows at once. The length is of the bytes as
                // written; one that disagrees desynchronises the connection.
                let mut line = format!(
                    "PUTSCRIPT {} {{{}+}}\r\n",
                    string(SCRIPT_NAME),
                    script.len()
                )
                .into_bytes();
                line.extend_from_slice(script.as_bytes());
                line.extend_from_slice(b"\r\n");
                Step::Need(vec![IoNeed::Write(line)])
            }
            SieveJob::Remove => match ours {
                None => self.logout(SieveOutcome::Removed {
                    caps: self.caps.clone(),
                    deleted: Deleted::NothingThere,
                }),
                // An active script cannot be deleted (RFC 5804 §2.10): switch it off first.
                Some(entry) if entry.active == Active::Yes => {
                    self.phase = Phase::Deactivate;
                    Step::Need(vec![IoNeed::Write(b"SETACTIVE \"\"\r\n".to_vec())])
                }
                Some(_) => self.delete(),
            },
        }
    }

    fn activate(&mut self) -> Step {
        let already = self
            .scripts
            .iter()
            .any(|s| s.ours() && s.active == Active::Yes);
        if already {
            return self.logout(SieveOutcome::Installed {
                caps: self.caps.clone(),
                displaced: None,
            });
        }
        self.phase = Phase::Activate;
        let line = format!("SETACTIVE {}\r\n", string(SCRIPT_NAME));
        Step::Need(vec![IoNeed::Write(line.into_bytes())])
    }

    fn delete(&mut self) -> Step {
        self.phase = Phase::Delete;
        let line = format!("DELETESCRIPT {}\r\n", string(SCRIPT_NAME));
        Step::Need(vec![IoNeed::Write(line.into_bytes())])
    }

    fn logout(&mut self, outcome: SieveOutcome) -> Step {
        self.outcome = Some(outcome);
        self.phase = Phase::Logout;
        Step::Need(vec![IoNeed::Write(b"LOGOUT\r\n".to_vec())])
    }
}

impl Machine for SieveSession {
    type Out = SieveOutcome;

    fn start(&mut self) -> Progress<SieveOutcome> {
        // The server speaks first, with its capabilities.
        Progress::Need(vec![IoNeed::Read])
    }

    fn feed(&mut self, ready: IoReady) -> Progress<SieveOutcome> {
        match ready {
            IoReady::Bytes(bytes) => {
                if self.phase == Phase::Finished {
                    return Progress::Failed(ProtoError::Malformed(
                        "bytes after the session finished".to_owned(),
                    ));
                }
                self.buf.extend_from_slice(&bytes);
                self.pump()
            }
            IoReady::TlsOpen if self.phase == Phase::AwaitTls => {
                self.phase = Phase::Reissued;
                Progress::Need(vec![IoNeed::Read])
            }
            // The server may close as soon as it has answered LOGOUT.
            IoReady::Eof if self.phase == Phase::Logout => match self.outcome.take() {
                Some(out) => {
                    self.phase = Phase::Finished;
                    Progress::Done(out)
                }
                None => self.fail(ProtoError::UnexpectedEof),
            },
            IoReady::Eof => self.fail(ProtoError::UnexpectedEof),
            // Nothing is half-written between commands, so stopping is simply stopping.
            IoReady::Interrupt => self.fail(ProtoError::UnexpectedEof),
            IoReady::TlsOpen | IoReady::Woke => Progress::Need(vec![IoNeed::Read]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    No,
    Bye,
    Data,
}

/// One `LISTSCRIPTS` line: a name, and `ACTIVE` after the one that is.
fn entry(line: &[Token]) -> Option<ScriptEntry> {
    let name = match line.first()? {
        Token::Text(_) => line[0].text(),
        _ => return None,
    };
    let active = match line.get(1) {
        Some(Token::Atom(a)) if a.eq_ignore_ascii_case("ACTIVE") => Active::Yes,
        _ => Active::No,
    };
    Some(ScriptEntry { name, active })
}

/// A quoted string for a command. Script names here are this client's own constant.
fn string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// RFC 4616: `\0 user \0 password`.
fn plain(user: &str, password: &str) -> Vec<u8> {
    let mut raw = Vec::with_capacity(user.len() + password.len() + 2);
    raw.push(0);
    raw.extend_from_slice(user.as_bytes());
    raw.push(0);
    raw.extend_from_slice(password.as_bytes());
    raw
}

/// `user=<addr>\x01auth=Bearer <token>\x01\x01`, the same bytes IMAP and SMTP send.
fn xoauth2(user: &str, token: &str) -> Vec<u8> {
    format!("user={user}\x01auth=Bearer {token}\x01\x01").into_bytes()
}
