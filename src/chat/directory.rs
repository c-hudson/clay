//! The per-session cache of one chat server: its channels, the users seen so far, roles,
//! and DM channels. Filled by the protocol task (`discord.rs`/`slack.rs`) and read
//! synchronously by `App` (for `/chat channels`, `/chat to`, rendering) through the
//! `Arc<std::sync::RwLock<_>>` in `ChatHandle` - never held across an `.await`.

use std::collections::HashMap;

use super::spec::{self, Spec, SpecHint};
use super::ChatKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    /// Discord announcement/news channel, Slack has none.
    News,
    /// A voice channel's text chat (Discord).
    VoiceText,
    Thread,
    Category,
    /// A Slack private channel (group). Discord private channels are plain `Text`.
    Private,
}

#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    /// Discord: category for a channel, parent channel for a thread.
    pub parent_id: Option<String>,
    pub position: i64,
    /// Slack: whether the bot is a member (it can only read/post where it is). Discord:
    /// unknown (None) unless computed.
    pub can_send: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct UserInfo {
    pub id: String,
    /// Login/handle (`alice`), used for `@alice` targeting.
    pub username: String,
    /// What to show (`Alice W.`): server nick, else global/real name, else username.
    pub display: String,
}

/// Where typed text goes: a channel, or a user (a DM, whose channel may not exist yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatTarget {
    pub channel_id: Option<String>,
    pub user_id: Option<String>,
    /// `#general`, `#general/thread-name`, or `@alice`.
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct ChatDirectory {
    pub kind: ChatKind,
    /// (id, name) of the server/workspace this world is attached to.
    pub server: Option<(String, String)>,
    pub bot_id: String,
    pub bot_name: String,
    pub channels: HashMap<String, ChannelInfo>,
    pub users: HashMap<String, UserInfo>,
    pub roles: HashMap<String, String>,
    /// DM channel id -> the other user's id.
    pub dm_channels: HashMap<String, String>,
    /// Where the most recent incoming message came from (`/chat to -`).
    pub last_inbound: Option<ChatTarget>,
}

impl ChatDirectory {
    pub fn new(kind: ChatKind) -> Self {
        ChatDirectory {
            kind,
            server: None,
            bot_id: String::new(),
            bot_name: String::new(),
            channels: HashMap::new(),
            users: HashMap::new(),
            roles: HashMap::new(),
            dm_channels: HashMap::new(),
            last_inbound: None,
        }
    }

    pub fn upsert_user(&mut self, user: UserInfo) {
        if user.id.is_empty() {
            return;
        }
        match self.users.get_mut(&user.id) {
            // Keep a better display name we already had (a server nick) when this
            // sighting carries less (e.g. a mention object without member data).
            Some(existing) => {
                if !user.username.is_empty() {
                    existing.username = user.username;
                }
                if !user.display.is_empty() {
                    existing.display = user.display;
                }
            }
            None => {
                self.users.insert(user.id.clone(), user);
            }
        }
    }

    /// `#name`, `#parent/thread` for a thread, `@user` for a DM channel, or the raw id.
    pub fn channel_label(&self, id: &str) -> String {
        if let Some(uid) = self.dm_channels.get(id) {
            return format!("@{}", self.user_handle(uid));
        }
        match self.channels.get(id) {
            Some(c) if c.kind == ChannelKind::Thread => {
                match c.parent_id.as_deref().and_then(|p| self.channels.get(p)) {
                    Some(parent) => format!("#{}/{}", parent.name, c.name),
                    None => format!("#{}", c.name),
                }
            }
            Some(c) => format!("#{}", c.name),
            None => format!("#{}", id),
        }
    }

    /// The name shown in `<...>` for a message author.
    pub fn user_display(&self, id: &str) -> String {
        match self.users.get(id) {
            Some(u) if !u.display.is_empty() => u.display.clone(),
            Some(u) if !u.username.is_empty() => u.username.clone(),
            _ => id.to_string(),
        }
    }

    /// The handle used in `@handle` labels/targets.
    pub fn user_handle(&self, id: &str) -> String {
        match self.users.get(id) {
            Some(u) if !u.username.is_empty() => u.username.clone(),
            Some(u) if !u.display.is_empty() => u.display.clone(),
            _ => id.to_string(),
        }
    }

    fn postable(c: &ChannelInfo) -> bool {
        c.kind != ChannelKind::Category
    }

    /// Resolve a channel spec to a channel id.
    pub fn resolve_channel(&self, spec: &Spec) -> Result<String, String> {
        if let Some(id) = &spec.id {
            // An id is trusted even if not (yet) in the cache - it may be a channel the
            // bot can post to but was never sent (e.g. a thread created while offline).
            return Ok(id.clone());
        }
        let hits: Vec<&ChannelInfo> = self
            .channels
            .values()
            .filter(|c| Self::postable(c) && c.name.to_lowercase() == spec.name)
            .collect();
        match hits.len() {
            1 => Ok(hits[0].id.clone()),
            0 => Err(self.not_found("channel", &spec.name, self.channels.values().filter(|c| Self::postable(c)).map(|c| c.name.clone()))),
            _ => Err(format!(
                "More than one channel is named '{}': {}. Use the id, e.g. /chat to {}",
                spec.name,
                hits.iter().map(|c| format!("{} ({})", self.channel_label(&c.id), c.id)).collect::<Vec<_>>().join(", "),
                hits[0].id
            )),
        }
    }

    /// Resolve a user spec to a user id, among the users seen this session.
    pub fn resolve_user(&self, spec: &Spec) -> Result<String, String> {
        if let Some(id) = &spec.id {
            return Ok(id.clone());
        }
        let hits: Vec<&UserInfo> = self
            .users
            .values()
            .filter(|u| u.username.to_lowercase() == spec.name || u.display.to_lowercase() == spec.name)
            .collect();
        match hits.len() {
            1 => Ok(hits[0].id.clone()),
            0 => Err(format!(
                "{} (Only people seen since connecting are known by name - use their numeric id otherwise.)",
                self.not_found("user", &spec.name, self.users.values().map(|u| u.username.clone()))
            )),
            _ => Err(format!(
                "More than one user matches '{}': {}. Use the id instead.",
                spec.name,
                hits.iter().map(|u| format!("{} ({})", u.username, u.id)).collect::<Vec<_>>().join(", ")
            )),
        }
    }

    fn not_found(&self, what: &str, name: &str, candidates: impl Iterator<Item = String>) -> String {
        let mut scored: Vec<(f64, String)> = candidates
            .map(|c| (strsim::jaro_winkler(&c.to_lowercase(), name), c))
            .filter(|(score, _)| *score > 0.75)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let suggestions: Vec<String> = scored.into_iter().take(3).map(|(_, c)| c).collect();
        if suggestions.is_empty() {
            format!("No {} named '{}'.", what, name)
        } else {
            format!("No {} named '{}' - did you mean {}?", what, name, suggestions.join(", "))
        }
    }

    /// Resolve a typed target (`/chat to ...`, the Send To setting) to a `ChatTarget`.
    /// `-` means "where the last message came from".
    pub fn resolve_target(&self, raw: &str) -> Result<ChatTarget, String> {
        if raw.trim() == "-" {
            return self.last_inbound.clone().ok_or_else(|| "No message has arrived yet to reply to.".to_string());
        }
        let spec = spec::parse(self.kind, raw).ok_or_else(|| "Empty target.".to_string())?;
        self.target_from_spec(&spec)
    }

    pub fn target_from_spec(&self, spec: &Spec) -> Result<ChatTarget, String> {
        match spec.hint {
            SpecHint::User => self.user_target(spec),
            SpecHint::Channel => self.channel_target(spec),
            SpecHint::Any => {
                // A known DM channel id or user id given bare still means that person.
                if let Some(id) = &spec.id {
                    if self.users.contains_key(id) && !self.channels.contains_key(id) {
                        return self.user_target(spec);
                    }
                }
                match self.channel_target(spec) {
                    Ok(t) => Ok(t),
                    Err(channel_err) => self.user_target(spec).map_err(|_| channel_err),
                }
            }
        }
    }

    fn channel_target(&self, spec: &Spec) -> Result<ChatTarget, String> {
        let id = self.resolve_channel(spec)?;
        Ok(ChatTarget { label: self.channel_label(&id), channel_id: Some(id), user_id: None })
    }

    fn user_target(&self, spec: &Spec) -> Result<ChatTarget, String> {
        let uid = self.resolve_user(spec)?;
        let channel_id = self.dm_channels.iter().find(|(_, u)| **u == uid).map(|(c, _)| c.clone());
        Ok(ChatTarget { label: format!("@{}", self.user_handle(&uid)), channel_id, user_id: Some(uid) })
    }

    /// Whether a message in `channel_id` passes the "Show Only" filter. DMs always pass;
    /// an empty filter passes everything. A thread passes when its parent does.
    pub fn passes_filter(&self, channel_id: &str, filter: &[Spec]) -> bool {
        if filter.is_empty() || self.dm_channels.contains_key(channel_id) {
            return true;
        }
        let matches = |id: &str| -> bool {
            let name = self.channels.get(id).map(|c| c.name.to_lowercase());
            filter.iter().any(|f| match (&f.id, &name) {
                (Some(fid), _) => fid == id,
                (None, Some(n)) => f.hint != SpecHint::User && *n == f.name,
                (None, None) => false,
            })
        };
        if matches(channel_id) {
            return true;
        }
        match self.channels.get(channel_id) {
            Some(c) if c.kind == ChannelKind::Thread => c.parent_id.as_deref().map(matches).unwrap_or(false),
            _ => false,
        }
    }

    /// Postable channels in display order: grouped under their category (by the
    /// category's position), then by position, then name. Threads are left out.
    pub fn sorted_channels(&self) -> Vec<&ChannelInfo> {
        let mut v: Vec<&ChannelInfo> = self
            .channels
            .values()
            .filter(|c| c.kind != ChannelKind::Category && c.kind != ChannelKind::Thread)
            .collect();
        let cat_pos = |c: &ChannelInfo| -> (i64, String) {
            match c.parent_id.as_deref().and_then(|p| self.channels.get(p)) {
                Some(cat) => (cat.position, cat.name.clone()),
                None => (-1, String::new()),
            }
        };
        v.sort_by(|a, b| {
            cat_pos(a).cmp(&cat_pos(b)).then(a.position.cmp(&b.position)).then(a.name.cmp(&b.name))
        });
        v
    }

    /// The category name for a channel, if any.
    pub fn category_name(&self, c: &ChannelInfo) -> Option<String> {
        c.parent_id
            .as_deref()
            .and_then(|p| self.channels.get(p))
            .filter(|p| p.kind == ChannelKind::Category)
            .map(|p| p.name.clone())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample() -> ChatDirectory {
        let mut d = ChatDirectory::new(ChatKind::Discord);
        let ch = |id: &str, name: &str, kind, parent: Option<&str>, pos| ChannelInfo {
            id: id.into(),
            name: name.into(),
            kind,
            parent_id: parent.map(|s| s.to_string()),
            position: pos,
            can_send: None,
        };
        for c in [
            ch("100000000000000001", "Text Channels", ChannelKind::Category, None, 0),
            ch("100000000000000002", "general", ChannelKind::Text, Some("100000000000000001"), 0),
            ch("100000000000000003", "random", ChannelKind::Text, Some("100000000000000001"), 1),
            ch("100000000000000004", "Other", ChannelKind::Category, None, 1),
            ch("100000000000000005", "general", ChannelKind::Text, Some("100000000000000004"), 0),
            ch("100000000000000006", "bugs", ChannelKind::Thread, Some("100000000000000003"), 0),
            ch("100000000000000007", "announcements", ChannelKind::News, None, 0),
        ] {
            d.channels.insert(c.id.clone(), c);
        }
        d.upsert_user(UserInfo { id: "200000000000000001".into(), username: "alice".into(), display: "Alice W".into() });
        d.upsert_user(UserInfo { id: "200000000000000002".into(), username: "bob".into(), display: String::new() });
        d.dm_channels.insert("300000000000000001".into(), "200000000000000001".into());
        d
    }

    fn sp(s: &str) -> Spec {
        spec::parse(ChatKind::Discord, s).unwrap()
    }

    #[test]
    fn labels() {
        let d = sample();
        assert_eq!(d.channel_label("100000000000000003"), "#random");
        assert_eq!(d.channel_label("100000000000000006"), "#random/bugs");
        assert_eq!(d.channel_label("300000000000000001"), "@alice");
        assert_eq!(d.user_display("200000000000000001"), "Alice W");
        assert_eq!(d.user_display("200000000000000002"), "bob");
        assert_eq!(d.user_display("999"), "999");
    }

    #[test]
    fn resolve_channels() {
        let d = sample();
        assert_eq!(d.resolve_channel(&sp("#Random")).unwrap(), "100000000000000003");
        // Duplicate names across categories are ambiguous and say so, with ids.
        let err = d.resolve_channel(&sp("general")).unwrap_err();
        assert!(err.contains("More than one") && err.contains("100000000000000005"), "{}", err);
        // The picker form disambiguates.
        assert_eq!(d.resolve_channel(&sp("general (100000000000000005)")).unwrap(), "100000000000000005");
        // A near miss suggests.
        let err = d.resolve_channel(&sp("#randm")).unwrap_err();
        assert!(err.contains("did you mean") && err.contains("random"), "{}", err);
        // Categories are never targets.
        assert!(d.resolve_channel(&sp("other")).is_err());
    }

    #[test]
    fn resolve_targets() {
        let d = sample();
        let t = d.resolve_target("@alice").unwrap();
        assert_eq!(t.label, "@alice");
        assert_eq!(t.user_id.as_deref(), Some("200000000000000001"));
        assert_eq!(t.channel_id.as_deref(), Some("300000000000000001"));
        // Bare name: channel first, then user.
        assert_eq!(d.resolve_target("bob").unwrap().label, "@bob");
        assert_eq!(d.resolve_target("random").unwrap().label, "#random");
        assert!(d.resolve_target("-").is_err());
        let t = d.resolve_target("#announcements").unwrap();
        assert_eq!(t.channel_id.as_deref(), Some("100000000000000007"));
    }

    #[test]
    fn filter() {
        let d = sample();
        let f = spec::parse_list(ChatKind::Discord, "#random");
        assert!(d.passes_filter("100000000000000003", &f));
        assert!(d.passes_filter("100000000000000006", &f), "thread of a shown channel is shown");
        assert!(!d.passes_filter("100000000000000002", &f));
        assert!(d.passes_filter("300000000000000001", &f), "DMs always shown");
        assert!(d.passes_filter("100000000000000002", &[]));
        let f = spec::parse_list(ChatKind::Discord, "100000000000000002");
        assert!(d.passes_filter("100000000000000002", &f));
        assert!(!d.passes_filter("100000000000000005", &f));
    }

    #[test]
    fn ordering() {
        let d = sample();
        let names: Vec<String> = d.sorted_channels().iter().map(|c| c.name.clone()).collect();
        assert_eq!(names, vec!["announcements", "general", "random", "general"]);
    }
}
