use std::fmt;

use serde::{Deserialize, Serialize};

pub const PROTO: u32 = 1;
pub const MIN_PROTO: u32 = 1;

const MAX_IDENTITY_LEN: usize = 255;

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidIdentity;

impl fmt::Display for InvalidIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid app identity")
    }
}

/// An Android package name or a macOS bundle identifier. The charset and shape keep it from
/// ever being a path traversal or an option flag, but consumers must still never use it as a
/// path or argv element. `Debug` never prints it, and no error built from one carries it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AppIdentity(String);

impl AppIdentity {
    pub fn new(raw: &str) -> Result<Self, InvalidIdentity> {
        let ok = raw.len() <= MAX_IDENTITY_LEN
            && raw
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphanumeric())
            && !raw.contains("..")
            && raw
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        if ok {
            Ok(Self(raw.to_owned()))
        } else {
            Err(InvalidIdentity)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AppIdentity {
    type Error = InvalidIdentity;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::new(&raw)
    }
}

impl From<AppIdentity> for String {
    fn from(id: AppIdentity) -> String {
        id.0
    }
}

impl fmt::Debug for AppIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AppIdentity(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protection {
    Full,
    Warn,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Running,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allowed,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Cooldown {
    Idle,
    Cooling { remaining_ms: u64 },
    Ready { expires_in_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    NotPending,
    Protected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayCount {
    pub day: u32,
    pub text: u32,
    pub domain: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEntry {
    pub identity: AppIdentity,
    pub removal: Cooldown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToDaemon {
    RequestDisarm,
    ConfirmDisarm,
    CancelDisarm,
    RaiseProtection,
    ListApps,
    RequestAddApp { identity: AppIdentity },
    RequestRemoveApp { identity: AppIdentity },
    ConfirmRemoveApp { identity: AppIdentity },
    CancelRemoveApp { identity: AppIdentity },
    Heartbeat,
    ScanEvent { verdict: Verdict, at_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToPanel {
    Status {
        protection: Protection,
        agent: AgentState,
        disarm: Cooldown,
    },
    Counts {
        days: Vec<DayCount>,
    },
    Apps {
        apps: Vec<AppEntry>,
    },
    Scan {
        verdict: Verdict,
        at_ms: u64,
    },
    Refused {
        reason: Refusal,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientRole {
    Panel,
    Agent,
}

/// A message type with a closed set of wire tags, so an unknown tag can be told apart from a
/// malformed body of a known one.
pub trait Wire: serde::de::DeserializeOwned {
    const TAGS: &'static [&'static str];
    fn tag(&self) -> &'static str;
}

impl Wire for ToDaemon {
    const TAGS: &'static [&'static str] = &[
        "request_disarm",
        "confirm_disarm",
        "cancel_disarm",
        "raise_protection",
        "list_apps",
        "request_add_app",
        "request_remove_app",
        "confirm_remove_app",
        "cancel_remove_app",
        "heartbeat",
        "scan_event",
    ];

    fn tag(&self) -> &'static str {
        match self {
            Self::RequestDisarm => "request_disarm",
            Self::ConfirmDisarm => "confirm_disarm",
            Self::CancelDisarm => "cancel_disarm",
            Self::RaiseProtection => "raise_protection",
            Self::ListApps => "list_apps",
            Self::RequestAddApp { .. } => "request_add_app",
            Self::RequestRemoveApp { .. } => "request_remove_app",
            Self::ConfirmRemoveApp { .. } => "confirm_remove_app",
            Self::CancelRemoveApp { .. } => "cancel_remove_app",
            Self::Heartbeat => "heartbeat",
            Self::ScanEvent { .. } => "scan_event",
        }
    }
}

impl ToDaemon {
    pub fn is_state_changing(&self) -> bool {
        matches!(
            self,
            Self::RequestDisarm
                | Self::ConfirmDisarm
                | Self::RequestAddApp { .. }
                | Self::RequestRemoveApp { .. }
                | Self::ConfirmRemoveApp { .. }
                | Self::CancelRemoveApp { .. }
        )
    }

    /// Dropping any of these would fail open, so the rate limiter never sees them.
    pub fn is_exempt_from_limits(&self) -> bool {
        matches!(
            self,
            Self::CancelDisarm | Self::RaiseProtection | Self::CancelRemoveApp { .. }
        )
    }

    pub fn allowed_for(&self, role: ClientRole) -> bool {
        let agent_only = matches!(self, Self::Heartbeat | Self::ScanEvent { .. });
        match role {
            ClientRole::Agent => agent_only,
            ClientRole::Panel => !agent_only,
        }
    }
}

impl Wire for ToPanel {
    const TAGS: &'static [&'static str] = &["status", "counts", "apps", "scan", "refused"];

    fn tag(&self) -> &'static str {
        match self {
            Self::Status { .. } => "status",
            Self::Counts { .. } => "counts",
            Self::Apps { .. } => "apps",
            Self::Scan { .. } => "scan",
            Self::Refused { .. } => "refused",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed<M> {
    Message(M),
    Unsupported,
    Malformed,
}

pub fn tag_of(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Tag {
        r#type: String,
    }
    serde_json::from_slice::<Tag>(body).ok().map(|t| t.r#type)
}

pub fn parse<M: Wire>(body: &[u8]) -> Parsed<M> {
    let Some(tag) = tag_of(body) else {
        return Parsed::Malformed;
    };
    match serde_json::from_slice::<M>(body) {
        Ok(message) => Parsed::Message(message),
        Err(_) if M::TAGS.contains(&tag.as_str()) => Parsed::Malformed,
        Err(_) => Parsed::Unsupported,
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename = "hello")]
pub struct Hello {
    pub proto: u32,
}

pub fn hello_body() -> Vec<u8> {
    serde_json::to_vec(&Hello { proto: PROTO }).expect("a fixed object serialises")
}
