//! Ephemeral, host/moderator-controlled rooms. Socket I/O belongs to the caller, never the actor.

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{
    sync::{Notify, mpsc, oneshot, watch},
    time::Instant,
};

const MAX_ROOMS: usize = 256;
const MAX_MEMBERS: usize = 32;
const MAILBOX_CAPACITY: usize = 64;
const RECENT_COMMANDS: usize = 256;
const MAX_EVENTS: usize = 100;
const EMPTY_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_POSITION_SECS: f64 = 7.0 * 24.0 * 60.0 * 60.0;
const COMMAND_BURST: f64 = 20.0;
const COMMANDS_PER_SECOND: f64 = 10.0;

static CLOCK_ORIGIN: OnceLock<Instant> = OnceLock::new();

#[derive(Clone)]
pub(crate) struct Rooms {
    registry: Arc<Mutex<HashMap<String, RoomHandle>>>,
    clock_origin: Instant,
    empty_ttl: Duration,
}

#[derive(Serialize)]
pub(crate) struct CreatedRoom {
    pub(crate) room_id: String,
    pub(crate) host_token: String,
}

#[derive(Clone)]
pub(crate) struct RoomHandle {
    sender: mpsc::Sender<Request>,
    departures: Arc<Departures>,
}

pub(crate) struct Membership {
    pub(crate) id: u64,
    pub(crate) role: Role,
    pub(crate) snapshots: watch::Receiver<Snapshot>,
    // Dropping a membership (including a cancelled join reply) releases its slot.
    _lease: Lease,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Host,
    Moderator,
    Guest,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Participant {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) role: Role,
    pub(crate) avatar: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Snapshot {
    pub(crate) incarnation: String,
    pub(crate) room_name: String,
    pub(crate) revision: u64,
    // Playback revision of the latest accepted load, even for the same video ID.
    pub(crate) media_revision: u64,
    pub(crate) video_id: Option<String>,
    pub(crate) playing: bool,
    pub(crate) playback_rate: f64,
    pub(crate) position_secs: f64,
    pub(crate) anchor_ms: u64,
    pub(crate) members: usize,
    pub(crate) participants: Vec<Participant>,
    pub(crate) events: Vec<FeedEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct FeedEntry {
    pub(crate) id: u64,
    pub(crate) at_ms: u64,
    pub(crate) member_id: u64,
    pub(crate) name: String,
    pub(crate) avatar: String,
    pub(crate) kind: FeedKind,
    pub(crate) text: Option<String>,
    pub(crate) video_id: Option<String>,
    pub(crate) position_secs: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FeedKind {
    Joined,
    Left,
    Message,
    Proposal,
    SetVideo,
    Play,
    Pause,
    Seek,
    SetRate,
    RoleChanged,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SocialAction {
    Message { text: String },
    ProposeVideo { video_id: String },
    SetModerator { member_id: u64, enabled: bool },
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    SetVideo { video_id: String },
    Play { position_secs: f64 },
    Pause { position_secs: f64 },
    Seek { position_secs: f64 },
    SetRate { playback_rate: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RoomError {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

impl fmt::Display for RoomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for RoomError {}

const INVALID_TOKEN: RoomError = RoomError {
    code: "invalid_token",
    message: "Invalid host token.",
};
const FORBIDDEN: RoomError = RoomError {
    code: "forbidden",
    message: "This member is not allowed to perform that action.",
};
const STALE_REVISION: RoomError = RoomError {
    code: "stale_revision",
    message: "Room state changed. Try again.",
};
const INVALID_COMMAND: RoomError = RoomError {
    code: "invalid_command",
    message: "Invalid room command.",
};
const BUSY: RoomError = RoomError {
    code: "busy",
    message: "Room is busy. Try again shortly.",
};
const UNAVAILABLE: RoomError = RoomError {
    code: "unavailable",
    message: "Room is unavailable.",
};

impl Rooms {
    pub(crate) fn new() -> Self {
        Self {
            registry: Arc::new(Mutex::new(HashMap::new())),
            clock_origin: *CLOCK_ORIGIN.get_or_init(Instant::now),
            empty_ttl: EMPTY_TTL,
        }
    }

    /// Same monotonic clock used by every production room's playback anchors.
    pub(crate) fn now_ms(&self) -> u64 {
        elapsed_ms(self.clock_origin)
    }

    /// Requires a running Tokio runtime, but never awaits while holding the registry lock.
    pub(crate) fn create(&self) -> Result<CreatedRoom, RoomError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| UNAVAILABLE)?;
        let mut registry = self.registry.lock().expect("room registry poisoned");
        registry.retain(|_, room| !room.sender.is_closed());
        if registry.len() >= MAX_ROOMS {
            return Err(BUSY);
        }

        let room_id = loop {
            let candidate = random_secret();
            if !registry.contains_key(&candidate) {
                break candidate;
            }
        };
        let host_token = random_secret();
        let (sender, receiver) = mpsc::channel(MAILBOX_CAPACITY);
        let departures = Arc::new(Departures::default());
        let actor = RoomActor::new(
            host_token.clone(),
            departures.clone(),
            self.clock_origin,
            self.empty_ttl,
        );
        registry.insert(room_id.clone(), RoomHandle { sender, departures });
        runtime.spawn(actor.run(receiver));
        Ok(CreatedRoom {
            room_id,
            host_token,
        })
    }

    pub(crate) fn get(&self, room_id: &str) -> Option<RoomHandle> {
        let mut registry = self.registry.lock().expect("room registry poisoned");
        if registry.get(room_id)?.sender.is_closed() {
            registry.remove(room_id);
            return None;
        }
        registry.get(room_id).cloned()
    }
}

impl RoomHandle {
    pub(crate) async fn join(&self, host_token: Option<String>) -> Result<Membership, RoomError> {
        let (reply, response) = oneshot::channel();
        self.enqueue(Request::Join { host_token, reply })?;
        response.await.map_err(|_| UNAVAILABLE)?
    }

    pub(crate) async fn command(
        &self,
        member_id: u64,
        id: String,
        revision: u64,
        action: Action,
    ) -> Result<u64, RoomError> {
        let (reply, response) = oneshot::channel();
        self.enqueue(Request::Command {
            member_id,
            id,
            revision,
            action,
            reply,
        })?;
        response.await.map_err(|_| UNAVAILABLE)?
    }

    pub(crate) async fn social(
        &self,
        member_id: u64,
        id: String,
        action: SocialAction,
    ) -> Result<u64, RoomError> {
        let (reply, response) = oneshot::channel();
        self.enqueue(Request::Social {
            member_id,
            id,
            action,
            reply,
        })?;
        response.await.map_err(|_| UNAVAILABLE)?
    }

    /// Synchronous, idempotent, and independent of mailbox capacity. Unknown IDs
    /// allocate nothing; pending departures are bounded by admitted membership.
    pub(crate) fn leave(&self, member_id: u64) {
        let leases = self
            .departures
            .leases
            .lock()
            .expect("member leases poisoned");
        if let Some(active) = leases.get(&member_id) {
            active.store(false, Ordering::Release);
            self.departures.changed.notify_one();
        }
    }

    fn enqueue(&self, request: Request) -> Result<(), RoomError> {
        self.sender.try_send(request).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => BUSY,
            mpsc::error::TrySendError::Closed(_) => UNAVAILABLE,
        })
    }
}

fn random_secret() -> String {
    // Thread-local rand RNG is OS-seeded. 256 random bits, encoded without padding.
    let bytes: [u8; 32] = rand::random();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    result
}

fn room_name() -> String {
    const ADJECTIVES: [&str; 8] = [
        "Sleepy", "Cosmic", "Cozy", "Wobbly", "Dreamy", "Jolly", "Fluffy", "Moonlit",
    ];
    const PLACES: [&str; 8] = [
        "Observatory",
        "Treehouse",
        "Burrow",
        "Cottage",
        "Teahouse",
        "Hideaway",
        "Nook",
        "Lighthouse",
    ];
    let draw: u8 = rand::random();
    format!(
        "The {} {}",
        ADJECTIVES[(draw % 8) as usize],
        PLACES[((draw / 8) % 8) as usize]
    )
}

fn member_name(id: u64, offset: u8, stride: u8) -> String {
    const ADJECTIVES: [&str; 16] = [
        "Sleepy", "Cosmic", "Wobbly", "Sneaky", "Dancing", "Fluffy", "Silly", "Dizzy", "Bouncy",
        "Fancy", "Jolly", "Fuzzy", "Dreamy", "Spicy", "Giggly", "Wiggly",
    ];
    const NOUNS: [&str; 16] = [
        "Waffle", "Goose", "Potato", "Otter", "Noodle", "Badger", "Pancake", "Penguin", "Pickle",
        "Muffin", "Llama", "Turnip", "Biscuit", "Raccoon", "Dumpling", "Puffin",
    ];
    // An odd stride permutes all 256 pairs; the room's random offset/stride
    // prevents every room starting with the same people while keeping names unique.
    let sequence = id - 1;
    let index = ((sequence % 256) * u64::from(stride) + u64::from(offset)) % 256;
    let adjective = ADJECTIVES[(index % 16) as usize];
    let noun = NOUNS[((index / 16) % 16) as usize];
    // Membership IDs never repeat. Number the noun after the vocabulary is used
    // up so even a long-lived room never recycles an author's two-word name.
    match sequence / 256 {
        0 => format!("{adjective} {noun}"),
        cycle => format!("{adjective} {noun}{}", cycle + 1),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.is_ascii()
}

fn valid_video_id(video_id: &str) -> bool {
    video_id.len() == 11
        && video_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn elapsed_ms(origin: Instant) -> u64 {
    Instant::now()
        .saturating_duration_since(origin)
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[derive(Default)]
struct Departures {
    leases: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    changed: Notify,
}

struct Lease {
    active: Arc<AtomicBool>,
    departures: Arc<Departures>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        self.departures.changed.notify_one();
    }
}

enum Request {
    Join {
        host_token: Option<String>,
        reply: oneshot::Sender<Result<Membership, RoomError>>,
    },
    Command {
        member_id: u64,
        id: String,
        revision: u64,
        action: Action,
        reply: oneshot::Sender<Result<u64, RoomError>>,
    },
    Social {
        member_id: u64,
        id: String,
        action: SocialAction,
        reply: oneshot::Sender<Result<u64, RoomError>>,
    },
}

struct Member {
    role: Role,
    name: String,
    avatar: String,
    active: Arc<AtomicBool>,
    tokens: f64,
    refilled_at: Instant,
    recent_social: VecDeque<AppliedSocial>,
}

impl Member {
    fn take_command_token(&mut self, now: Instant) -> bool {
        self.tokens = (self.tokens
            + now
                .saturating_duration_since(self.refilled_at)
                .as_secs_f64()
                * COMMANDS_PER_SECOND)
            .min(COMMAND_BURST);
        self.refilled_at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

struct AppliedSocial {
    id: String,
    action: SocialAction,
    event_id: u64,
}

struct AppliedCommand {
    member_id: u64,
    id: String,
    expected_revision: u64,
    action: Action,
    applied_revision: u64,
}

struct RoomActor {
    host_token: String,
    name_offset: u8,
    name_stride: u8,
    departures: Arc<Departures>,
    members: HashMap<u64, Member>,
    next_member_id: u64,
    snapshot: Snapshot,
    snapshots: watch::Sender<Snapshot>,
    recent: VecDeque<AppliedCommand>,
    next_event_id: u64,
    clock_origin: Instant,
    empty_since: Option<Instant>,
    empty_ttl: Duration,
}

impl RoomActor {
    fn new(
        host_token: String,
        departures: Arc<Departures>,
        clock_origin: Instant,
        empty_ttl: Duration,
    ) -> Self {
        let snapshot = Snapshot {
            incarnation: random_secret(),
            room_name: room_name(),
            revision: 0,
            media_revision: 0,
            video_id: None,
            playing: false,
            playback_rate: 1.0,
            position_secs: 0.0,
            anchor_ms: elapsed_ms(clock_origin),
            members: 0,
            participants: Vec::new(),
            events: Vec::new(),
        };
        let (snapshots, _) = watch::channel(snapshot.clone());
        Self {
            host_token,
            name_offset: rand::random(),
            name_stride: rand::random::<u8>() | 1,
            departures,
            members: HashMap::new(),
            next_member_id: 1,
            snapshot,
            snapshots,
            recent: VecDeque::new(),
            next_event_id: 1,
            clock_origin,
            empty_since: Some(Instant::now()),
            empty_ttl,
        }
    }

    async fn run(mut self, mut receiver: mpsc::Receiver<Request>) {
        loop {
            self.remove_departed();
            let deadline = self.empty_since.map(|since| since + self.empty_ttl);
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }

            tokio::select! {
                // The deadline cannot be starved by traffic to an empty room.
                biased;
                _ = async {
                    match deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending::<()>().await,
                    }
                } => break,
                _ = self.departures.changed.notified() => {},
                request = receiver.recv() => {
                    let Some(request) = request else { break };
                    // A lease can be released while recv was pending.
                    self.remove_departed();
                    match request {
                        Request::Join { host_token, reply } => {
                            if !reply.is_closed() {
                                // If the caller cancels concurrently, send drops the
                                // returned Membership and its lease performs cleanup.
                                let result = self.join(host_token);
                                let _ = reply.send(result);
                            }
                        }
                        Request::Command { member_id, id, revision, action, reply } => {
                            // An enqueued command may apply even if its caller goes away.
                            let result = self.command(member_id, id, revision, action);
                            let _ = reply.send(result);
                        }
                        Request::Social { member_id, id, action, reply } => {
                            let result = self.social(member_id, id, action);
                            let _ = reply.send(result);
                        }
                    }
                }
            }
        }
        // Dropping receiver closes all registry/external handles and waiting replies.
    }

    fn join(&mut self, host_token: Option<String>) -> Result<Membership, RoomError> {
        let role = match host_token {
            None => Role::Guest,
            Some(token) if token == self.host_token => Role::Host,
            Some(_) => return Err(INVALID_TOKEN),
        };
        if self.members.len() >= MAX_MEMBERS {
            return Err(BUSY);
        }
        let id = self.next_member_id;
        let next_member_id = id.checked_add(1).ok_or(UNAVAILABLE)?;
        // Preflight before admitting a lease; exhausted event IDs cannot leave
        // an unreachable active membership behind.
        self.next_event_id.checked_add(1).ok_or(UNAVAILABLE)?;
        self.next_member_id = next_member_id;
        let active = Arc::new(AtomicBool::new(true));
        self.departures
            .leases
            .lock()
            .expect("member leases poisoned")
            .insert(id, active.clone());
        self.members.insert(
            id,
            Member {
                role,
                name: member_name(id, self.name_offset, self.name_stride),
                avatar: random_secret()[..16].to_owned(),
                active: active.clone(),
                tokens: COMMAND_BURST,
                refilled_at: Instant::now(),
                recent_social: VecDeque::new(),
            },
        );
        self.empty_since = None;
        self.record_event(id, FeedKind::Joined, None, None, None)?;
        self.update_participants();
        self.publish();
        Ok(Membership {
            id,
            role,
            snapshots: self.snapshots.subscribe(),
            _lease: Lease {
                active,
                departures: self.departures.clone(),
            },
        })
    }

    fn remove_departed(&mut self) {
        let mut departed: Vec<_> = self
            .members
            .iter()
            .filter_map(|(&id, member)| (!member.active.load(Ordering::Acquire)).then_some(id))
            .collect();
        if departed.is_empty() {
            return;
        }
        departed.sort_unstable();
        for id in departed {
            let _ = self.record_event(id, FeedKind::Left, None, None, None);
            self.members.remove(&id);
        }
        self.departures
            .leases
            .lock()
            .expect("member leases poisoned")
            .retain(|id, _| self.members.contains_key(id));
        if self.members.is_empty() {
            self.empty_since = Some(Instant::now());
        }
        self.update_participants();
        // Membership updates preserve the playback anchor, even while playing.
        self.publish();
    }

    fn update_participants(&mut self) {
        let mut participants: Vec<_> = self
            .members
            .iter()
            .map(|(&id, member)| Participant {
                id,
                name: member.name.clone(),
                role: member.role,
                avatar: member.avatar.clone(),
            })
            .collect();
        participants.sort_unstable_by_key(|participant| participant.id);

        self.snapshot.members = participants.len();
        self.snapshot.participants = participants;
    }

    fn command(
        &mut self,
        member_id: u64,
        id: String,
        revision: u64,
        action: Action,
    ) -> Result<u64, RoomError> {
        // Authorization precedes payload validation, revision checking and dedupe.
        let member = self.members.get_mut(&member_id).ok_or(FORBIDDEN)?;
        if member.role == Role::Guest || !member.active.load(Ordering::Acquire) {
            return Err(FORBIDDEN);
        }
        if !member.take_command_token(Instant::now()) {
            return Err(BUSY);
        }
        if !valid_id(&id) || !action.is_valid() {
            return Err(INVALID_COMMAND);
        }
        if self.snapshot.video_id.is_none() && !matches!(action, Action::SetVideo { .. }) {
            return Err(INVALID_COMMAND);
        }
        if let Some(applied) = self
            .recent
            .iter()
            .find(|applied| applied.member_id == member_id && applied.id == id)
        {
            if applied.expected_revision != revision || applied.action != action {
                return Err(INVALID_COMMAND);
            }
            return Ok(applied.applied_revision);
        }
        if revision != self.snapshot.revision {
            return Err(STALE_REVISION);
        }
        let next_revision = revision.checked_add(1).ok_or(UNAVAILABLE)?;
        let now_ms = elapsed_ms(self.clock_origin);
        let (kind, text, video_id, position_secs) = match &action {
            Action::SetVideo { video_id } => {
                (FeedKind::SetVideo, None, Some(video_id.clone()), Some(0.0))
            }
            Action::Play { position_secs } => (FeedKind::Play, None, None, Some(*position_secs)),
            Action::Pause { position_secs } => (FeedKind::Pause, None, None, Some(*position_secs)),
            Action::Seek { position_secs } => (FeedKind::Seek, None, None, Some(*position_secs)),
            Action::SetRate { playback_rate } => (
                FeedKind::SetRate,
                Some(format!("{playback_rate}x")),
                None,
                None,
            ),
        };
        // Native scrubbing arrives as a combined position + playing intent.
        // Compare against the old timeline, not its anchored (possibly stale) position.
        let mut inferred_seek = None;
        let mut record_action = true;
        if let Action::Play { position_secs } | Action::Pause { position_secs } = &action {
            let projected_position = if self.snapshot.playing {
                let elapsed_secs = now_ms.saturating_sub(self.snapshot.anchor_ms) as f64 / 1000.0;
                (self.snapshot.position_secs + elapsed_secs * self.snapshot.playback_rate)
                    .min(MAX_POSITION_SECS)
            } else {
                self.snapshot.position_secs
            };
            if (*position_secs - projected_position).abs() > 0.75 {
                inferred_seek = Some(*position_secs);
                record_action = matches!(action, Action::Play { .. }) != self.snapshot.playing;
            }
        }
        // A combined seek/state change must not append half an activity on overflow.
        let event_count = u64::from(inferred_seek.is_some()) + u64::from(record_action);
        self.next_event_id
            .checked_add(event_count)
            .ok_or(UNAVAILABLE)?;
        if let Some(position_secs) = inferred_seek {
            self.record_event(member_id, FeedKind::Seek, None, None, Some(position_secs))?;
        }
        if record_action {
            self.record_event(member_id, kind, text, video_id, position_secs)?;
        }
        match &action {
            Action::SetVideo { video_id } => {
                self.snapshot.media_revision = next_revision;
                self.snapshot.video_id = Some(video_id.clone());
                self.snapshot.playing = false;
                self.snapshot.playback_rate = 1.0;
                self.snapshot.position_secs = 0.0;
            }
            Action::Play { position_secs } => {
                self.snapshot.playing = true;
                self.snapshot.position_secs = *position_secs;
            }
            Action::Pause { position_secs } => {
                self.snapshot.playing = false;
                self.snapshot.position_secs = *position_secs;
            }
            Action::Seek { position_secs } => {
                self.snapshot.position_secs = *position_secs;
            }
            Action::SetRate { playback_rate } => {
                // Preserve continuity: elapsed playback belongs to the old rate.
                if self.snapshot.playing {
                    let elapsed_secs =
                        now_ms.saturating_sub(self.snapshot.anchor_ms) as f64 / 1000.0;
                    self.snapshot.position_secs = (self.snapshot.position_secs
                        + elapsed_secs * self.snapshot.playback_rate)
                        .min(MAX_POSITION_SECS);
                }
                self.snapshot.playback_rate = *playback_rate;
            }
        }
        self.snapshot.anchor_ms = now_ms;
        self.snapshot.revision = next_revision;
        if self.recent.len() == RECENT_COMMANDS {
            self.recent.pop_front();
        }
        self.recent.push_back(AppliedCommand {
            member_id,
            id,
            expected_revision: revision,
            action,
            applied_revision: next_revision,
        });
        self.publish();
        Ok(next_revision)
    }

    fn social(
        &mut self,
        member_id: u64,
        id: String,
        mut action: SocialAction,
    ) -> Result<u64, RoomError> {
        let member = self.members.get_mut(&member_id).ok_or(FORBIDDEN)?;
        if !member.active.load(Ordering::Acquire)
            || (matches!(action, SocialAction::SetModerator { .. }) && member.role != Role::Host)
        {
            return Err(FORBIDDEN);
        }
        if !member.take_command_token(Instant::now()) {
            return Err(BUSY);
        }
        if !valid_id(&id) {
            return Err(INVALID_COMMAND);
        }
        match &mut action {
            SocialAction::Message { text } => {
                let trimmed = text.trim();
                if trimmed.is_empty() || trimmed.len() > 2000 || trimmed.chars().count() > 500 {
                    return Err(INVALID_COMMAND);
                }
                *text = trimmed.to_owned();
            }
            SocialAction::ProposeVideo { video_id } if !valid_video_id(video_id) => {
                return Err(INVALID_COMMAND);
            }
            _ => {}
        }
        let member = self.members.get(&member_id).ok_or(FORBIDDEN)?;
        if let Some(applied) = member.recent_social.iter().find(|applied| applied.id == id) {
            if applied.action != action {
                return Err(INVALID_COMMAND);
            }
            return Ok(applied.event_id);
        }
        if let SocialAction::SetModerator {
            member_id: target_id,
            ..
        } = &action
        {
            let target = self.members.get(target_id).ok_or(INVALID_COMMAND)?;
            if !target.active.load(Ordering::Acquire) {
                return Err(INVALID_COMMAND);
            }
            if target.role == Role::Host {
                return Err(FORBIDDEN);
            }
        }

        let (kind, text, video_id) = match &action {
            SocialAction::Message { text } => (FeedKind::Message, Some(text.clone()), None),
            SocialAction::ProposeVideo { video_id } => {
                (FeedKind::Proposal, None, Some(video_id.clone()))
            }
            SocialAction::SetModerator {
                member_id: target_id,
                enabled,
            } => {
                let target = &self.members[target_id];
                let role = if *enabled { "moderator" } else { "guest" };
                (
                    FeedKind::RoleChanged,
                    Some(format!("{} is now a {role}.", target.name)),
                    None,
                )
            }
        };
        let event_id = self.record_event(member_id, kind, text, video_id, None)?;
        if let SocialAction::SetModerator {
            member_id: target_id,
            enabled,
        } = &action
        {
            self.members
                .get_mut(target_id)
                .expect("validated target")
                .role = if *enabled {
                Role::Moderator
            } else {
                Role::Guest
            };
            self.update_participants();
        }
        let recent = &mut self
            .members
            .get_mut(&member_id)
            .expect("validated author")
            .recent_social;
        if recent.len() == RECENT_COMMANDS {
            recent.pop_front();
        }
        recent.push_back(AppliedSocial {
            id,
            action,
            event_id,
        });
        self.publish();
        Ok(event_id)
    }

    fn record_event(
        &mut self,
        member_id: u64,
        kind: FeedKind,
        text: Option<String>,
        video_id: Option<String>,
        position_secs: Option<f64>,
    ) -> Result<u64, RoomError> {
        let id = self.next_event_id;
        let next_id = id.checked_add(1).ok_or(UNAVAILABLE)?;
        let member = self.members.get(&member_id).ok_or(FORBIDDEN)?;
        let event = FeedEntry {
            id,
            at_ms: elapsed_ms(self.clock_origin),
            member_id,
            name: member.name.clone(),
            avatar: member.avatar.clone(),
            kind,
            text,
            video_id,
            position_secs,
        };
        self.next_event_id = next_id;
        if self.snapshot.events.len() == MAX_EVENTS {
            self.snapshot.events.remove(0);
        }
        self.snapshot.events.push(event);
        Ok(id)
    }

    fn publish(&self) {
        // Retain the latest snapshot even when there are currently no receivers.
        self.snapshots.send_replace(self.snapshot.clone());
    }
}

impl Action {
    fn is_valid(&self) -> bool {
        match self {
            Self::SetVideo { video_id } => valid_video_id(video_id),
            Self::Play { position_secs }
            | Self::Pause { position_secs }
            | Self::Seek { position_secs } => {
                position_secs.is_finite() && (0.0..=MAX_POSITION_SECS).contains(position_secs)
            }
            Self::SetRate { playback_rate } => {
                playback_rate.is_finite() && (0.25..=4.0).contains(playback_rate)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rooms(ttl: Duration) -> Rooms {
        Rooms {
            registry: Arc::new(Mutex::new(HashMap::new())),
            clock_origin: Instant::now(),
            empty_ttl: ttl,
        }
    }

    async fn host_room(rooms: &Rooms) -> (RoomHandle, Membership) {
        let created = rooms.create().unwrap();
        let room = rooms.get(&created.room_id).unwrap();
        let host = room.join(Some(created.host_token)).await.unwrap();
        (room, host)
    }

    fn video() -> Action {
        Action::SetVideo {
            video_id: "dQw4w9WgXcQ".into(),
        }
    }

    async fn settle() {
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn production_rooms_share_the_process_clock_and_clone_registry() {
        let rooms = Rooms::new();
        let clone = rooms.clone();
        let before = rooms.now_ms();
        let created = rooms.create().unwrap();
        assert_eq!(created.room_id.len(), 64);
        assert_eq!(created.host_token.len(), 64);
        assert_ne!(created.room_id, created.host_token);
        let room = clone.get(&created.room_id).unwrap();
        let guest = room.join(None).await.unwrap();
        let initial = guest.snapshots.borrow().clone();
        assert!(initial.anchor_ms >= before);
        assert!(initial.anchor_ms <= rooms.now_ms());
        assert_ne!(initial.incarnation, created.host_token);
        let (other, host) = host_room(&Rooms::new()).await;
        assert_eq!(
            other.command(host.id, "anchor".into(), 0, video()).await,
            Ok(1)
        );
        assert!(host.snapshots.borrow().anchor_ms >= initial.anchor_ms);
        assert!(host.snapshots.borrow().anchor_ms <= rooms.now_ms());
    }

    #[tokio::test]
    async fn participant_snapshots_have_stable_public_avatars_and_remove_departures() {
        let rooms = Rooms::new();
        let created = rooms.create().unwrap();
        let room = rooms.get(&created.room_id).unwrap();
        let host = room.join(Some(created.host_token.clone())).await.unwrap();
        let first = host.snapshots.borrow().participants[0].clone();

        assert_eq!(first.id, host.id);
        assert_eq!(first.role, Role::Host);
        assert_eq!(first.avatar.len(), 16);
        assert!(first.avatar.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first.avatar, created.host_token);

        let guest = room.join(None).await.unwrap();
        let joined = host.snapshots.borrow().clone();
        assert_eq!(joined.members, joined.participants.len());
        assert_eq!(joined.participants[0], first);
        assert_eq!(joined.participants[1].id, guest.id);
        assert_eq!(joined.participants[1].role, Role::Guest);
        assert_ne!(joined.participants[1].avatar, first.avatar);
        assert!(
            !serde_json::to_string(&joined)
                .unwrap()
                .contains(&created.host_token)
        );

        room.command(host.id, "load".into(), 0, video())
            .await
            .unwrap();
        assert_eq!(host.snapshots.borrow().participants, joined.participants);
        room.leave(guest.id);
        settle().await;

        let departed = host.snapshots.borrow().clone();
        assert_eq!(departed.members, 1);
        assert_eq!(departed.participants, vec![first]);
        assert_eq!(departed.revision, 1);
    }

    #[tokio::test]
    async fn authentication_precedes_validation_revision_and_dedupe() {
        let rooms = Rooms::new();
        let (room, host) = host_room(&rooms).await;
        assert_eq!(host.role, Role::Host);
        assert_eq!(
            room.join(Some("wrong".into())).await.err(),
            Some(INVALID_TOKEN)
        );
        let guest = room.join(None).await.unwrap();
        assert_eq!(guest.role, Role::Guest);
        assert_eq!(
            room.command(host.id, "valid".into(), 0, video()).await,
            Ok(1)
        );
        for (id, revision, action) in [
            ("valid", 0, video()),
            (
                "",
                u64::MAX,
                Action::Play {
                    position_secs: f64::NAN,
                },
            ),
        ] {
            assert_eq!(
                room.command(guest.id, id.into(), revision, action).await,
                Err(FORBIDDEN)
            );
        }
        assert_eq!(
            room.command(u64::MAX, "valid".into(), 0, video()).await,
            Err(FORBIDDEN)
        );
        room.leave(host.id);
        assert_eq!(
            room.command(host.id, "valid".into(), 0, video()).await,
            Err(FORBIDDEN)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn playing_anchors_are_monotonic_and_membership_does_not_reanchor() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, host) = host_room(&rooms).await;
        assert_eq!(
            room.command(host.id, "video".into(), 0, video()).await,
            Ok(1)
        );
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(
            room.command(
                host.id,
                "play".into(),
                1,
                Action::Play {
                    position_secs: 12.5
                }
            )
            .await,
            Ok(2)
        );
        let playing = host.snapshots.borrow().clone();
        assert!(playing.playing);
        assert_eq!(playing.anchor_ms, 2000);
        assert_eq!(playing.position_secs, 12.5);

        tokio::time::advance(Duration::from_secs(3)).await;
        let guest = room.join(None).await.unwrap();
        let joined = guest.snapshots.borrow().clone();
        assert_eq!(joined.anchor_ms, playing.anchor_ms);
        assert_eq!(joined.revision, playing.revision);
        let current_position = joined.position_secs
            + (elapsed_ms(rooms.clock_origin) - joined.anchor_ms) as f64 / 1000.0;
        assert_eq!(current_position, 15.5);
        drop(guest);
        settle().await;
        assert_eq!(host.snapshots.borrow().anchor_ms, 2000);

        assert_eq!(
            room.command(
                host.id,
                "seek".into(),
                2,
                Action::Seek {
                    position_secs: 40.0
                }
            )
            .await,
            Ok(3)
        );
        assert!(host.snapshots.borrow().playing);
        assert_eq!(host.snapshots.borrow().anchor_ms, 5000);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            room.command(
                host.id,
                "pause".into(),
                3,
                Action::Pause {
                    position_secs: 41.0
                }
            )
            .await,
            Ok(4)
        );
        assert!(!host.snapshots.borrow().playing);
        assert_eq!(host.snapshots.borrow().anchor_ms, 6000);
        assert_eq!(
            room.command(
                host.id,
                "seek-paused".into(),
                4,
                Action::Seek { position_secs: 2.0 }
            )
            .await,
            Ok(5)
        );
        assert!(!host.snapshots.borrow().playing);
        assert_eq!(
            room.command(host.id, "reset".into(), 5, video()).await,
            Ok(6)
        );
        assert_eq!(host.snapshots.borrow().position_secs, 0.0);
        assert!(!host.snapshots.borrow().playing);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_changes_reanchor_using_the_old_rate_without_advancing_paused_playback() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, host) = host_room(&rooms).await;
        assert_eq!(host.snapshots.borrow().playback_rate, 1.0);
        assert_eq!(
            room.command(host.id, "load".into(), 0, video()).await,
            Ok(1)
        );
        assert_eq!(
            room.command(
                host.id,
                "play".into(),
                1,
                Action::Play {
                    position_secs: 10.0
                }
            )
            .await,
            Ok(2)
        );

        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(
            room.command(
                host.id,
                "double".into(),
                2,
                Action::SetRate { playback_rate: 2.0 }
            )
            .await,
            Ok(3)
        );
        let double = host.snapshots.borrow().clone();
        assert_eq!(double.position_secs, 12.0);
        assert_eq!(double.anchor_ms, 2000);
        assert_eq!(double.playback_rate, 2.0);
        assert!(double.playing);
        assert_eq!(double.media_revision, 1);

        tokio::time::advance(Duration::from_secs(3)).await;
        assert_eq!(
            room.command(
                host.id,
                "double".into(),
                2,
                Action::SetRate { playback_rate: 2.0 }
            )
            .await,
            Ok(3)
        );
        assert_eq!(*host.snapshots.borrow(), double);
        let guest = room.join(None).await.unwrap();
        let joined = guest.snapshots.borrow().clone();
        assert_eq!(joined.anchor_ms, double.anchor_ms);
        assert_eq!(joined.position_secs, double.position_secs);
        assert_eq!(joined.playback_rate, double.playback_rate);
        assert_eq!(joined.revision, double.revision);
        drop(guest);
        settle().await;
        let departed = host.snapshots.borrow().clone();
        assert_eq!(departed.participants, double.participants);
        assert_eq!(departed.revision, double.revision);
        assert_eq!(departed.media_revision, double.media_revision);
        assert_eq!(departed.anchor_ms, double.anchor_ms);
        assert_eq!(departed.position_secs, double.position_secs);
        assert_eq!(departed.playback_rate, double.playback_rate);
        assert_eq!(departed.events.len(), double.events.len() + 2);
        assert_eq!(departed.events.last().unwrap().kind, FeedKind::Left);
        assert_eq!(
            room.command(
                host.id,
                "half".into(),
                3,
                Action::SetRate { playback_rate: 0.5 }
            )
            .await,
            Ok(4)
        );
        assert_eq!(host.snapshots.borrow().position_secs, 18.0);
        assert_eq!(host.snapshots.borrow().anchor_ms, 5000);
        assert!(host.snapshots.borrow().playing);
        assert_eq!(host.snapshots.borrow().media_revision, 1);

        tokio::time::advance(Duration::from_secs(4)).await;
        assert_eq!(
            room.command(
                host.id,
                "pause".into(),
                4,
                Action::Pause {
                    position_secs: 20.0
                }
            )
            .await,
            Ok(5)
        );
        tokio::time::advance(Duration::from_secs(10)).await;
        assert_eq!(
            room.command(
                host.id,
                "paused-rate".into(),
                5,
                Action::SetRate { playback_rate: 4.0 }
            )
            .await,
            Ok(6)
        );
        assert_eq!(host.snapshots.borrow().position_secs, 20.0);
        assert_eq!(host.snapshots.borrow().anchor_ms, 19000);
        assert_eq!(host.snapshots.borrow().playback_rate, 4.0);
        assert!(!host.snapshots.borrow().playing);
        assert_eq!(host.snapshots.borrow().media_revision, 1);

        assert_eq!(
            room.command(
                host.id,
                "near-limit".into(),
                6,
                Action::Play {
                    position_secs: MAX_POSITION_SECS - 1.0
                }
            )
            .await,
            Ok(7)
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            room.command(
                host.id,
                "clamped-rate".into(),
                7,
                Action::SetRate { playback_rate: 2.0 }
            )
            .await,
            Ok(8)
        );
        assert_eq!(host.snapshots.borrow().position_secs, MAX_POSITION_SECS);
        assert_eq!(host.snapshots.borrow().anchor_ms, 20000);
        assert!(host.snapshots.borrow().playing);
        assert_eq!(host.snapshots.borrow().media_revision, 1);

        assert_eq!(
            room.command(host.id, "reload".into(), 8, video()).await,
            Ok(9)
        );
        assert_eq!(host.snapshots.borrow().playback_rate, 1.0);
        assert_eq!(host.snapshots.borrow().position_secs, 0.0);
        assert!(!host.snapshots.borrow().playing);
        assert_eq!(host.snapshots.borrow().media_revision, 9);
    }

    #[tokio::test]
    async fn rate_commands_obey_authorization_bounds_revision_and_dedupe() {
        let rooms = Rooms::new();
        let (room, host) = host_room(&rooms).await;
        assert_eq!(
            room.command(host.id, "load".into(), 0, video()).await,
            Ok(1)
        );
        let guest = room.join(None).await.unwrap();
        let initial = host.snapshots.borrow().clone();
        assert_eq!(
            room.command(
                guest.id,
                "forbidden-rate".into(),
                u64::MAX,
                Action::SetRate {
                    playback_rate: f64::NAN
                }
            )
            .await,
            Err(FORBIDDEN)
        );
        for playback_rate in [
            -1.0,
            0.0,
            0.249,
            4.001,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert_eq!(
                room.command(
                    host.id,
                    "invalid-rate".into(),
                    1,
                    Action::SetRate { playback_rate }
                )
                .await,
                Err(INVALID_COMMAND)
            );
            assert_eq!(*host.snapshots.borrow(), initial);
        }
        assert_eq!(
            room.command(
                host.id,
                "stale-rate".into(),
                0,
                Action::SetRate { playback_rate: 1.0 }
            )
            .await,
            Err(STALE_REVISION)
        );
        assert_eq!(*host.snapshots.borrow(), initial);

        let minimum = Action::SetRate {
            playback_rate: 0.25,
        };
        assert_eq!(
            room.command(host.id, "minimum".into(), 1, minimum.clone())
                .await,
            Ok(2)
        );
        let slow = host.snapshots.borrow().clone();
        assert_eq!(slow.playback_rate, 0.25);
        assert_eq!(slow.media_revision, 1);
        assert_eq!(
            room.command(host.id, "minimum".into(), 1, minimum.clone())
                .await,
            Ok(2)
        );
        assert_eq!(*host.snapshots.borrow(), slow);
        assert_eq!(
            room.command(
                host.id,
                "minimum".into(),
                1,
                Action::SetRate { playback_rate: 4.0 }
            )
            .await,
            Err(INVALID_COMMAND)
        );
        assert_eq!(
            room.command(host.id, "minimum".into(), 2, minimum.clone())
                .await,
            Err(INVALID_COMMAND)
        );
        assert_eq!(*host.snapshots.borrow(), slow);

        assert_eq!(
            room.command(
                host.id,
                "maximum".into(),
                2,
                Action::SetRate { playback_rate: 4.0 }
            )
            .await,
            Ok(3)
        );
        let fast = host.snapshots.borrow().clone();
        assert_eq!(fast.playback_rate, 4.0);
        assert_eq!(fast.media_revision, 1);
        assert_eq!(
            room.command(host.id, "minimum".into(), 1, minimum).await,
            Ok(2)
        );
        assert_eq!(
            room.command(
                host.id,
                "stale-after-rate".into(),
                2,
                Action::SetRate { playback_rate: 1.0 }
            )
            .await,
            Err(STALE_REVISION)
        );
        assert_eq!(*host.snapshots.borrow(), fast);
    }

    #[tokio::test]
    async fn media_revision_changes_only_on_accepted_loads_including_the_same_video() {
        let rooms = Rooms::new();
        let (room, host) = host_room(&rooms).await;
        assert_eq!(host.snapshots.borrow().media_revision, 0);
        assert_eq!(
            room.command(host.id, "load".into(), 0, video()).await,
            Ok(1)
        );
        assert_eq!(host.snapshots.borrow().media_revision, 1);

        // Native controls can produce zero-position anchors without reloading media.
        for (revision, action) in [
            (1, Action::Play { position_secs: 0.0 }),
            (2, Action::Seek { position_secs: 0.0 }),
            (3, Action::Pause { position_secs: 0.0 }),
        ] {
            assert_eq!(
                room.command(host.id, revision.to_string(), revision, action)
                    .await,
                Ok(revision + 1)
            );
            assert_eq!(host.snapshots.borrow().revision, revision + 1);
            assert_eq!(host.snapshots.borrow().media_revision, 1);
        }
        let guest = room.join(None).await.unwrap();
        assert_eq!(guest.snapshots.borrow().media_revision, 1);
        assert_eq!(guest.snapshots.borrow().revision, 4);
        drop(guest);
        settle().await;
        assert_eq!(host.snapshots.borrow().media_revision, 1);
        assert_eq!(host.snapshots.borrow().revision, 4);

        // Selecting the identical video is a new load, not an ordinary pause/seek.
        assert_eq!(
            room.command(host.id, "reload".into(), 4, video()).await,
            Ok(5)
        );
        let reloaded = host.snapshots.borrow().clone();
        assert_eq!(reloaded.media_revision, 5);
        assert_eq!(reloaded.video_id, Some("dQw4w9WgXcQ".into()));
        assert!(!reloaded.playing);
        assert_eq!(reloaded.position_secs, 0.0);
        assert_eq!(
            room.command(host.id, "reload".into(), 4, video()).await,
            Ok(5)
        );
        assert_eq!(*host.snapshots.borrow(), reloaded);

        let other_video = Action::SetVideo {
            video_id: "abcdefghijk".into(),
        };
        assert_eq!(
            room.command(host.id, "stale-load".into(), 4, other_video.clone())
                .await,
            Err(STALE_REVISION)
        );
        assert_eq!(
            room.command(host.id, "reload".into(), 4, other_video.clone())
                .await,
            Err(INVALID_COMMAND)
        );
        assert_eq!(
            room.command(
                host.id,
                "invalid-load".into(),
                5,
                Action::SetVideo {
                    video_id: "invalid".into()
                }
            )
            .await,
            Err(INVALID_COMMAND)
        );
        assert_eq!(*host.snapshots.borrow(), reloaded);

        assert_eq!(
            room.command(host.id, "other-load".into(), 5, other_video)
                .await,
            Ok(6)
        );
        assert_eq!(host.snapshots.borrow().media_revision, 6);
        assert_eq!(host.snapshots.borrow().revision, 6);
    }

    #[tokio::test]
    async fn stale_duplicate_and_conflicting_ids_do_not_mutate_state() {
        let rooms = Rooms::new();
        let (room, host) = host_room(&rooms).await;
        assert_eq!(
            room.command(host.id, "first".into(), 0, video()).await,
            Ok(1)
        );
        let first = host.snapshots.borrow().clone();
        assert_eq!(
            room.command(host.id, "first".into(), 0, video()).await,
            Ok(1)
        );
        assert_eq!(*host.snapshots.borrow(), first);
        assert_eq!(
            room.command(host.id, "stale".into(), 0, video()).await,
            Err(STALE_REVISION)
        );
        assert_eq!(
            room.command(
                host.id,
                "second".into(),
                1,
                Action::Play { position_secs: 0.0 }
            )
            .await,
            Ok(2)
        );
        let second = host.snapshots.borrow().clone();
        assert_eq!(
            room.command(host.id, "first".into(), 0, video()).await,
            Ok(1)
        );
        assert_eq!(
            room.command(host.id, "first".into(), 1, video()).await,
            Err(INVALID_COMMAND)
        );
        assert_eq!(
            room.command(
                host.id,
                "first".into(),
                0,
                Action::Seek { position_secs: 0.0 }
            )
            .await,
            Err(INVALID_COMMAND)
        );
        assert_eq!(*host.snapshots.borrow(), second);
    }

    #[tokio::test(start_paused = true)]
    async fn playback_requires_a_selected_video() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, mut host) = host_room(&rooms).await;
        let initial = host.snapshots.borrow_and_update().clone();
        tokio::time::advance(Duration::from_secs(1)).await;
        for action in [
            Action::Play {
                position_secs: 10.0,
            },
            Action::Pause {
                position_secs: 10.0,
            },
            Action::Seek {
                position_secs: 10.0,
            },
            Action::SetRate { playback_rate: 2.0 },
        ] {
            assert_eq!(
                room.command(host.id, "playback".into(), 0, action).await,
                Err(INVALID_COMMAND)
            );
            assert_eq!(*host.snapshots.borrow(), initial);
            assert!(!host.snapshots.has_changed().unwrap());
        }

        assert_eq!(
            room.command(host.id, "video".into(), 0, video()).await,
            Ok(1)
        );
        // Rejected IDs are not recorded as applied; playback is now allowed.
        for (revision, action) in [
            (
                1,
                Action::Play {
                    position_secs: 10.0,
                },
            ),
            (
                2,
                Action::Pause {
                    position_secs: 10.0,
                },
            ),
            (
                3,
                Action::Seek {
                    position_secs: 20.0,
                },
            ),
        ] {
            let id = if revision == 1 {
                "playback".into()
            } else {
                revision.to_string()
            };
            assert_eq!(
                room.command(host.id, id, revision, action).await,
                Ok(revision + 1)
            );
        }
        assert_eq!(host.snapshots.borrow().revision, 4);
        assert!(!host.snapshots.borrow().playing);
        assert_eq!(host.snapshots.borrow().position_secs, 20.0);
    }

    #[tokio::test]
    async fn invalid_ids_videos_and_positions_are_rejected() {
        let rooms = Rooms::new();
        let (room, host) = host_room(&rooms).await;
        for id in ["".to_string(), "x".repeat(65), "é".to_string()] {
            assert_eq!(
                room.command(host.id, id, 0, video()).await,
                Err(INVALID_COMMAND)
            );
        }
        for video_id in ["short", "abcdefghijkl", "dQw4w9WgXc!", "éQw4w9WgXc"] {
            assert_eq!(
                room.command(
                    host.id,
                    "bad".into(),
                    0,
                    Action::SetVideo {
                        video_id: video_id.into()
                    }
                )
                .await,
                Err(INVALID_COMMAND)
            );
        }
        assert_eq!(host.snapshots.borrow().revision, 0);
        assert_eq!(
            room.command(host.id, "video".into(), 0, video()).await,
            Ok(1)
        );
        for position_secs in [-1.0, f64::NAN, f64::INFINITY, MAX_POSITION_SECS + 1.0] {
            assert_eq!(
                room.command(host.id, "bad".into(), 1, Action::Seek { position_secs })
                    .await,
                Err(INVALID_COMMAND)
            );
        }
        assert_eq!(host.snapshots.borrow().revision, 1);
        assert_eq!(
            room.command(
                host.id,
                "max".into(),
                1,
                Action::Seek {
                    position_secs: MAX_POSITION_SECS
                }
            )
            .await,
            Ok(2)
        );
    }

    #[tokio::test]
    async fn membership_is_capped_and_leases_release_slots_without_revision_changes() {
        let rooms = Rooms::new();
        let (room, host) = host_room(&rooms).await;
        let mut guests = Vec::new();
        for _ in 1..MAX_MEMBERS {
            guests.push(room.join(None).await.unwrap());
        }
        assert_eq!(host.snapshots.borrow().members, MAX_MEMBERS);
        assert_eq!(room.join(None).await.err(), Some(BUSY));
        let guest = guests.pop().unwrap();
        room.leave(guest.id);
        room.leave(guest.id);
        let replacement = room.join(None).await.unwrap();
        assert_ne!(guest.id, replacement.id);
        drop(guest); // Cannot release a newer member's slot.
        settle().await;
        assert_eq!(host.snapshots.borrow().members, MAX_MEMBERS);
        drop(guests);
        drop(replacement);
        settle().await;
        assert_eq!(host.snapshots.borrow().members, 1);
        assert_eq!(host.snapshots.borrow().revision, 0);
    }

    #[tokio::test]
    async fn watch_coalesces_to_latest_complete_state() {
        let rooms = Rooms::new();
        let (room, mut host) = host_room(&rooms).await;
        assert_eq!(
            room.command(host.id, "video".into(), 0, video()).await,
            Ok(1)
        );
        host.snapshots.borrow_and_update();
        for revision in 1..11 {
            assert_eq!(
                room.command(
                    host.id,
                    revision.to_string(),
                    revision,
                    Action::Seek {
                        position_secs: revision as f64
                    }
                )
                .await,
                Ok(revision + 1)
            );
        }
        host.snapshots.changed().await.unwrap();
        let snapshot = host.snapshots.borrow_and_update().clone();
        assert_eq!(snapshot.revision, 11);
        assert_eq!(snapshot.position_secs, 10.0);
        assert_eq!(snapshot.members, 1);
        assert!(!host.snapshots.has_changed().unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn command_rate_limit_has_burst_and_refill() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, host) = host_room(&rooms).await;
        for revision in 0..20 {
            assert_eq!(
                room.command(host.id, revision.to_string(), revision, video())
                    .await,
                Ok(revision + 1)
            );
        }
        assert_eq!(
            room.command(host.id, "over".into(), 20, video()).await,
            Err(BUSY)
        );
        tokio::time::advance(Duration::from_millis(100)).await;
        assert_eq!(
            room.command(host.id, "refilled".into(), 20, video()).await,
            Ok(21)
        );
        assert_eq!(
            room.command(host.id, "over-again".into(), 21, video())
                .await,
            Err(BUSY)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn never_joined_and_departed_rooms_expire_and_handles_close() {
        let ttl = Duration::from_secs(10);
        let rooms = test_rooms(ttl);
        let created = rooms.create().unwrap();
        let room = rooms.get(&created.room_id).unwrap();
        settle().await;
        tokio::time::advance(ttl).await;
        settle().await;
        assert!(room.sender.is_closed());
        assert!(rooms.get(&created.room_id).is_none());
        assert_eq!(room.join(None).await.err(), Some(UNAVAILABLE));

        let created = rooms.create().unwrap();
        let room = rooms.get(&created.room_id).unwrap();
        let member = room.join(None).await.unwrap();
        tokio::time::advance(ttl * 2).await;
        assert!(!room.sender.is_closed());
        drop(member);
        settle().await;
        tokio::time::advance(ttl - Duration::from_millis(1)).await;
        settle().await;
        assert!(!room.sender.is_closed());
        // Rejoin cancels empty expiry; a subsequent departure restarts it.
        let member = room.join(None).await.unwrap();
        tokio::time::advance(ttl).await;
        assert!(!room.sender.is_closed());
        drop(member);
        settle().await;
        tokio::time::advance(ttl).await;
        settle().await;
        assert!(rooms.get(&created.room_id).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn registry_cap_and_lazy_create_cleanup() {
        let rooms = test_rooms(Duration::from_secs(1));
        let first = rooms.create().unwrap();
        for _ in 1..MAX_ROOMS {
            rooms.create().unwrap();
        }
        assert_eq!(rooms.create().err(), Some(BUSY));
        settle().await;
        tokio::time::advance(Duration::from_secs(1)).await;
        settle().await;
        rooms.create().unwrap();
        assert!(rooms.get(&first.room_id).is_none());
        assert_eq!(rooms.registry.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn departures_survive_a_full_mailbox_and_cancelled_join() {
        let rooms = Rooms::new();
        let (room, host) = host_room(&rooms).await;
        // No await while filling: the current-thread actor cannot drain the queue.
        let mut responses = Vec::new();
        for _ in 0..MAILBOX_CAPACITY {
            let (reply, response) = oneshot::channel();
            room.enqueue(Request::Command {
                member_id: host.id,
                id: "queued".into(),
                revision: 0,
                action: video(),
                reply,
            })
            .unwrap();
            responses.push(response);
        }
        assert_eq!(
            room.command(host.id, "full".into(), 0, video()).await,
            Err(BUSY)
        );
        room.leave(host.id);
        room.leave(u64::MAX);
        for response in responses {
            assert_eq!(response.await.unwrap(), Err(FORBIDDEN));
        }
        assert_eq!(host.snapshots.borrow().members, 0);
        assert_eq!(host.snapshots.borrow().revision, 0);

        let (reply, response) = oneshot::channel();
        room.enqueue(Request::Join {
            host_token: None,
            reply,
        })
        .unwrap();
        drop(response);
        settle().await;
        assert_eq!(host.snapshots.borrow().members, 0);

        // Cancellation after actor admission also drops the lease in the reply.
        let (reply, response) = oneshot::channel();
        room.enqueue(Request::Join {
            host_token: None,
            reply,
        })
        .unwrap();
        settle().await;
        assert_eq!(host.snapshots.borrow().members, 1);
        drop(response);
        settle().await;
        assert_eq!(host.snapshots.borrow().members, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn dedupe_cache_is_bounded_and_old_ids_obey_revision_checks() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, host) = host_room(&rooms).await;
        for revision in 0..=RECENT_COMMANDS as u64 {
            tokio::time::advance(Duration::from_millis(100)).await;
            assert_eq!(
                room.command(host.id, revision.to_string(), revision, video())
                    .await,
                Ok(revision + 1)
            );
        }
        assert_eq!(
            room.command(host.id, "0".into(), 0, video()).await,
            Err(STALE_REVISION)
        );
        assert_eq!(room.command(host.id, "1".into(), 1, video()).await, Ok(2));
    }

    #[tokio::test(start_paused = true)]
    async fn native_combined_seek_activity_projects_the_old_timeline_and_retries_once() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, host) = host_room(&rooms).await;
        room.command(host.id, "load".into(), 0, video())
            .await
            .unwrap();
        let loaded = host.snapshots.borrow().clone();

        room.command(
            host.id,
            "normal-play".into(),
            1,
            Action::Play { position_secs: 0.0 },
        )
        .await
        .unwrap();
        let playing = host.snapshots.borrow().clone();
        assert_eq!(
            &playing.events[loaded.events.len()..],
            &[FeedEntry {
                id: loaded.events.last().unwrap().id + 1,
                at_ms: 0,
                member_id: host.id,
                name: loaded.participants[0].name.clone(),
                avatar: loaded.participants[0].avatar.clone(),
                kind: FeedKind::Play,
                text: None,
                video_id: None,
                position_secs: Some(0.0),
            }]
        );
        room.command(
            host.id,
            "double-rate".into(),
            2,
            Action::SetRate { playback_rate: 2.0 },
        )
        .await
        .unwrap();

        for (index, delay_ms, action, kinds) in [
            (
                0,
                3000,
                Action::Play { position_secs: 6.0 },
                vec![FeedKind::Play],
            ),
            (
                1,
                0,
                Action::Play {
                    position_secs: 6.75,
                },
                vec![FeedKind::Play],
            ),
            (
                2,
                1000,
                Action::Play {
                    position_secs: 20.0,
                },
                vec![FeedKind::Seek],
            ),
            (
                3,
                1000,
                Action::Pause {
                    position_secs: 30.0,
                },
                vec![FeedKind::Seek, FeedKind::Pause],
            ),
            (
                4,
                1000,
                Action::Pause {
                    position_secs: 40.0,
                },
                vec![FeedKind::Seek],
            ),
            (
                5,
                1000,
                Action::Play {
                    position_secs: 50.0,
                },
                vec![FeedKind::Seek, FeedKind::Play],
            ),
            (
                6,
                1000,
                Action::Seek {
                    position_secs: 50.0,
                },
                vec![FeedKind::Seek],
            ),
            (
                7,
                1000,
                Action::Pause {
                    position_secs: 52.0,
                },
                vec![FeedKind::Pause],
            ),
            (
                8,
                1000,
                Action::Pause {
                    position_secs: 10.0,
                },
                vec![FeedKind::Seek],
            ),
        ] {
            tokio::time::advance(Duration::from_millis(delay_ms)).await;
            let before = host.snapshots.borrow().clone();
            let revision = before.revision;
            let id = format!("native-{index}");
            assert_eq!(
                room.command(host.id, id.clone(), revision, action.clone())
                    .await,
                Ok(revision + 1)
            );
            let after = host.snapshots.borrow().clone();
            let position_secs = match action {
                Action::Play { position_secs }
                | Action::Pause { position_secs }
                | Action::Seek { position_secs } => position_secs,
                _ => unreachable!(),
            };

            assert_eq!(after.revision, revision + 1);
            assert_eq!(after.media_revision, loaded.media_revision);
            assert_eq!(after.anchor_ms, elapsed_ms(rooms.clock_origin));
            assert_eq!(after.position_secs, position_secs);
            assert_eq!(after.playback_rate, 2.0);
            assert_eq!(
                after.playing,
                match action {
                    Action::Play { .. } => true,
                    Action::Pause { .. } => false,
                    _ => before.playing,
                }
            );

            let expected: Vec<_> = kinds
                .into_iter()
                .enumerate()
                .map(|(offset, kind)| FeedEntry {
                    id: before.events.last().unwrap().id + 1 + offset as u64,
                    at_ms: after.anchor_ms,
                    member_id: host.id,
                    name: loaded.participants[0].name.clone(),
                    avatar: loaded.participants[0].avatar.clone(),
                    kind,
                    text: None,
                    video_id: None,
                    position_secs: Some(position_secs),
                })
                .collect();
            assert_eq!(&after.events[before.events.len()..], &expected);

            assert_eq!(
                room.command(host.id, id, revision, action).await,
                Ok(revision + 1)
            );
            assert_eq!(*host.snapshots.borrow(), after);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn host_grants_and_revokes_all_playback_authority_without_reanchoring() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, host) = host_room(&rooms).await;
        let guest = room.join(None).await.unwrap();
        let other = room.join(None).await.unwrap();
        let initial = host.snapshots.borrow().clone();
        let grant = SocialAction::SetModerator {
            member_id: guest.id,
            enabled: true,
        };

        for author in [guest.id, other.id, u64::MAX] {
            assert_eq!(
                room.social(author, "grant".into(), grant.clone()).await,
                Err(FORBIDDEN)
            );
        }
        for enabled in [true, false] {
            assert_eq!(
                room.social(
                    host.id,
                    "host".into(),
                    SocialAction::SetModerator {
                        member_id: host.id,
                        enabled,
                    }
                )
                .await,
                Err(FORBIDDEN)
            );
        }
        assert_eq!(
            room.social(
                host.id,
                "missing".into(),
                SocialAction::SetModerator {
                    member_id: u64::MAX,
                    enabled: true,
                }
            )
            .await,
            Err(INVALID_COMMAND)
        );
        assert_eq!(*host.snapshots.borrow(), initial);

        tokio::time::advance(Duration::from_secs(1)).await;
        let granted = room
            .social(host.id, "grant".into(), grant.clone())
            .await
            .unwrap();
        let moderator = host.snapshots.borrow().clone();
        assert_eq!(moderator.participants[1].role, Role::Moderator);
        assert_eq!(moderator.participants[1].name, initial.participants[1].name);
        assert_eq!(moderator.revision, initial.revision);
        assert_eq!(moderator.media_revision, initial.media_revision);
        assert_eq!(moderator.anchor_ms, initial.anchor_ms);
        assert_eq!(moderator.room_name, initial.room_name);
        assert_eq!(moderator.events.last().unwrap().kind, FeedKind::RoleChanged);
        assert_eq!(moderator.events.last().unwrap().member_id, host.id);
        assert!(
            moderator
                .events
                .last()
                .unwrap()
                .text
                .as_ref()
                .unwrap()
                .contains(&initial.participants[1].name)
        );
        assert_eq!(
            room.social(
                guest.id,
                "delegate".into(),
                SocialAction::SetModerator {
                    member_id: other.id,
                    enabled: true,
                }
            )
            .await,
            Err(FORBIDDEN)
        );

        let actions = [
            video(),
            Action::Play { position_secs: 0.0 },
            Action::Pause { position_secs: 0.0 },
            Action::Seek { position_secs: 3.0 },
            Action::SetRate { playback_rate: 1.5 },
        ];
        for (revision, action) in actions.iter().enumerate() {
            assert_eq!(
                room.command(
                    guest.id,
                    format!("action-{revision}"),
                    revision as u64,
                    action.clone()
                )
                .await,
                Ok(revision as u64 + 1)
            );
        }
        let playback = host.snapshots.borrow().clone();
        assert_eq!(
            playback
                .events
                .iter()
                .rev()
                .take(5)
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![
                FeedKind::SetRate,
                FeedKind::Seek,
                FeedKind::Pause,
                FeedKind::Play,
                FeedKind::SetVideo
            ]
        );
        assert!(
            playback
                .events
                .iter()
                .rev()
                .take(5)
                .all(|event| event.member_id == guest.id)
        );
        assert_eq!(
            playback.events.last().unwrap().text.as_deref(),
            Some("1.5x")
        );

        tokio::time::advance(Duration::from_secs(1)).await;
        room.social(
            host.id,
            "revoke".into(),
            SocialAction::SetModerator {
                member_id: guest.id,
                enabled: false,
            },
        )
        .await
        .unwrap();
        let revoked = host.snapshots.borrow().clone();
        assert_eq!(revoked.participants[1].role, Role::Guest);
        assert_eq!(revoked.revision, playback.revision);
        assert_eq!(revoked.media_revision, playback.media_revision);
        assert_eq!(revoked.anchor_ms, playback.anchor_ms);
        assert_eq!(revoked.room_name, playback.room_name);

        for (revision, action) in actions.iter().enumerate() {
            assert_eq!(
                room.command(
                    guest.id,
                    format!("action-{revision}"),
                    revision as u64,
                    action.clone()
                )
                .await,
                Err(FORBIDDEN)
            );
        }
        assert_eq!(
            room.social(host.id, "grant".into(), grant.clone()).await,
            Ok(granted)
        );
        assert_eq!(*host.snapshots.borrow(), revoked); // Retry must not re-grant.

        room.leave(guest.id);
        settle().await;
        let departed = host.snapshots.borrow().clone();
        assert_eq!(
            room.social(host.id, "grant".into(), grant).await,
            Ok(granted)
        );
        assert_eq!(*host.snapshots.borrow(), departed);
        assert_eq!(
            room.social(
                host.id,
                "new-grant".into(),
                SocialAction::SetModerator {
                    member_id: guest.id,
                    enabled: true,
                }
            )
            .await,
            Err(INVALID_COMMAND)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn messages_and_proposals_capture_shared_identity_and_preserve_playback() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, host) = host_room(&rooms).await;
        room.command(host.id, "load".into(), 0, video())
            .await
            .unwrap();
        room.command(
            host.id,
            "play".into(),
            1,
            Action::Play {
                position_secs: 10.0,
            },
        )
        .await
        .unwrap();
        let guest = room.join(None).await.unwrap();
        let before = guest.snapshots.borrow().clone();
        let author = before.participants[1].clone();

        assert_eq!(host.snapshots.borrow().participants, before.participants);
        assert_ne!(before.participants[0].name, author.name);
        assert_eq!(author.name.split_whitespace().count(), 2);
        assert!(before.room_name.starts_with("The "));

        tokio::time::advance(Duration::from_secs(5)).await;
        let message_id = room
            .social(
                guest.id,
                "chat".into(),
                SocialAction::Message {
                    text: "  hello <script> & friends  ".into(),
                },
            )
            .await
            .unwrap();
        let proposal_id = room
            .social(
                guest.id,
                "proposal".into(),
                SocialAction::ProposeVideo {
                    video_id: "abcdefghijk".into(),
                },
            )
            .await
            .unwrap();
        let social = host.snapshots.borrow().clone();

        assert_eq!(social.revision, before.revision);
        assert_eq!(social.media_revision, before.media_revision);
        assert_eq!(social.anchor_ms, before.anchor_ms);
        assert_eq!(social.position_secs, before.position_secs);
        assert_eq!(social.video_id, before.video_id);
        assert_eq!(social.playing, before.playing);
        assert_eq!(social.room_name, before.room_name);
        assert_eq!(
            serde_json::to_value(&social.events[social.events.len() - 2]).unwrap(),
            serde_json::json!({
                "id": message_id, "at_ms": 5000, "member_id": guest.id,
                "name": author.name, "avatar": author.avatar, "kind": "message",
                "text": "hello <script> & friends", "video_id": null, "position_secs": null,
            })
        );
        assert_eq!(
            serde_json::to_value(social.events.last().unwrap()).unwrap(),
            serde_json::json!({
                "id": proposal_id, "at_ms": 5000, "member_id": guest.id,
                "name": author.name, "avatar": author.avatar, "kind": "proposal",
                "text": null, "video_id": "abcdefghijk", "position_secs": null,
            })
        );

        room.leave(guest.id);
        settle().await;
        let departed = host.snapshots.borrow().clone();
        assert_eq!(departed.participants.len(), 1);
        assert_eq!(&departed.events[..social.events.len()], &social.events);
        let left = departed.events.last().unwrap();
        assert_eq!(left.kind, FeedKind::Left);
        assert_eq!(left.member_id, author.id);
        assert_eq!(left.name, author.name);
        assert_eq!(left.avatar, author.avatar);
        assert_eq!(departed.revision, before.revision);
        assert_eq!(departed.anchor_ms, before.anchor_ms);

        drop(host);
        settle().await;
        let returning = room.join(None).await.unwrap();
        assert_eq!(returning.snapshots.borrow().room_name, before.room_name);
        assert!(
            returning
                .snapshots
                .borrow()
                .events
                .iter()
                .any(|event| event.id == message_id)
        );
        assert_eq!(
            room.social(
                guest.id,
                "chat".into(),
                SocialAction::Message {
                    text: "hello <script> & friends".into(),
                }
            )
            .await,
            Err(FORBIDDEN)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn social_validation_idempotency_and_member_rate_limits_are_independent_of_playback() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, host) = host_room(&rooms).await;
        let guest = room.join(None).await.unwrap();
        let message = SocialAction::Message {
            text: "hello".into(),
        };
        let first_id = room
            .social(guest.id, "same".into(), message.clone())
            .await
            .unwrap();
        let first = host.snapshots.borrow().clone();

        assert_eq!(
            room.social(guest.id, "same".into(), message.clone()).await,
            Ok(first_id)
        );
        assert_eq!(*host.snapshots.borrow(), first);
        assert_eq!(
            room.social(
                guest.id,
                "same".into(),
                SocialAction::Message {
                    text: "different".into(),
                }
            )
            .await,
            Err(INVALID_COMMAND)
        );
        let host_id = room
            .social(host.id, "same".into(), message.clone())
            .await
            .unwrap();
        assert_ne!(first_id, host_id);
        assert_eq!(
            host.snapshots.borrow().events.last().unwrap().member_id,
            host.id
        );
        assert_eq!(
            room.command(host.id, "same".into(), 0, video()).await,
            Ok(1)
        );

        let before = host.snapshots.borrow().clone();
        for id in ["".to_owned(), "x".repeat(65), "é".into()] {
            assert_eq!(
                room.social(guest.id, id, message.clone()).await,
                Err(INVALID_COMMAND)
            );
        }
        for text in [" \n\t".into(), "x".repeat(501), "😀".repeat(501)] {
            assert_eq!(
                room.social(guest.id, "bad".into(), SocialAction::Message { text })
                    .await,
                Err(INVALID_COMMAND)
            );
        }
        for video_id in ["short", "abcdefghijkl", "dQw4w9WgXc!", "éQw4w9WgXc"] {
            assert_eq!(
                room.social(
                    guest.id,
                    "bad".into(),
                    SocialAction::ProposeVideo {
                        video_id: video_id.into(),
                    }
                )
                .await,
                Err(INVALID_COMMAND)
            );
        }
        assert_eq!(*host.snapshots.borrow(), before);

        let boundary_id = room
            .social(
                guest.id,
                "boundary".into(),
                SocialAction::Message {
                    text: format!("  {}  ", "😀".repeat(500)),
                },
            )
            .await
            .unwrap();
        let boundary = host.snapshots.borrow().clone();
        assert_eq!(
            boundary.events.last().unwrap().text.as_ref().unwrap().len(),
            2000
        );
        assert_eq!(boundary.events.last().unwrap().id, boundary_id);

        tokio::time::advance(Duration::from_secs(2)).await;
        for _ in 0..20 {
            assert_eq!(
                room.social(guest.id, "same".into(), message.clone()).await,
                Ok(first_id)
            );
        }
        assert_eq!(
            room.social(guest.id, "over".into(), message.clone()).await,
            Err(BUSY)
        );
        assert_eq!(*host.snapshots.borrow(), boundary);
        tokio::time::advance(Duration::from_millis(100)).await;
        assert!(
            room.social(guest.id, "refill".into(), message.clone())
                .await
                .is_ok()
        );
        assert_eq!(
            room.social(guest.id, "over-again".into(), message).await,
            Err(BUSY)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn social_and_playback_share_tokens_but_not_authors_or_retry_windows() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, host) = host_room(&rooms).await;
        let guest = room.join(None).await.unwrap();
        room.social(
            host.id,
            "grant".into(),
            SocialAction::SetModerator {
                member_id: guest.id,
                enabled: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            room.command(host.id, "shared-id".into(), 0, video()).await,
            Ok(1)
        );
        let loaded = host.snapshots.borrow().clone();

        // An authorized different author must not borrow somebody else's ack.
        assert_eq!(
            room.command(guest.id, "shared-id".into(), 0, video()).await,
            Err(STALE_REVISION)
        );
        assert_eq!(*host.snapshots.borrow(), loaded);
        assert_eq!(
            room.command(guest.id, "shared-id".into(), 1, video()).await,
            Ok(2)
        );
        let moderator_load = host.snapshots.borrow().clone();
        assert_eq!(moderator_load.events.last().unwrap().member_id, guest.id);
        assert_eq!(
            room.command(host.id, "shared-id".into(), 0, video()).await,
            Ok(1)
        );
        assert_eq!(*host.snapshots.borrow(), moderator_load);

        tokio::time::advance(Duration::from_secs(2)).await;
        for index in 0..10 {
            assert!(
                room.social(
                    host.id,
                    format!("message-{index}"),
                    SocialAction::Message {
                        text: "hello".into(),
                    }
                )
                .await
                .is_ok()
            );
            assert_eq!(
                room.command(host.id, "shared-id".into(), 0, video()).await,
                Ok(1)
            );
        }
        let full = host.snapshots.borrow().clone();
        assert_eq!(
            room.social(
                host.id,
                "over-social".into(),
                SocialAction::Message {
                    text: "hello".into(),
                }
            )
            .await,
            Err(BUSY)
        );
        assert_eq!(
            room.command(host.id, "over-playback".into(), 2, video())
                .await,
            Err(BUSY)
        );
        assert_eq!(*host.snapshots.borrow(), full);
    }

    #[tokio::test(start_paused = true)]
    async fn retained_history_survives_coalescing_and_social_dedupe_has_its_own_bound() {
        let rooms = test_rooms(EMPTY_TTL);
        let (room, mut host) = host_room(&rooms).await;
        let guest = room.join(None).await.unwrap();
        room.command(host.id, "load".into(), 0, video())
            .await
            .unwrap();
        let before = host.snapshots.borrow_and_update().clone();
        let message = SocialAction::Message {
            text: "hello".into(),
        };
        let mut ids = Vec::new();

        for index in 0..=RECENT_COMMANDS {
            tokio::time::advance(Duration::from_millis(100)).await;
            ids.push(
                room.social(guest.id, index.to_string(), message.clone())
                    .await
                    .unwrap(),
            );
        }
        host.snapshots.changed().await.unwrap();
        let coalesced = host.snapshots.borrow_and_update().clone();

        assert_eq!(coalesced.events.len(), MAX_EVENTS);
        assert_eq!(
            coalesced
                .events
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            ids[ids.len() - MAX_EVENTS..]
        );
        assert!(
            coalesced
                .events
                .windows(2)
                .all(|pair| pair[0].at_ms <= pair[1].at_ms)
        );
        assert_eq!(coalesced.revision, before.revision);
        assert_eq!(coalesced.media_revision, before.media_revision);
        assert_eq!(coalesced.anchor_ms, before.anchor_ms);
        assert_eq!(coalesced.room_name, before.room_name);
        assert!(!host.snapshots.has_changed().unwrap());

        assert_eq!(
            room.command(host.id, "load".into(), 0, video()).await,
            Ok(1)
        );
        assert_eq!(*host.snapshots.borrow(), coalesced);
        assert_eq!(
            room.social(guest.id, "1".into(), message.clone()).await,
            Ok(ids[1])
        );
        assert_eq!(*host.snapshots.borrow(), coalesced); // Dedupe outlives visible history.
        let recycled = room.social(guest.id, "0".into(), message).await.unwrap();
        assert!(recycled > *ids.last().unwrap()); // Outside the bounded retry window.
        assert_eq!(host.snapshots.borrow().events.len(), MAX_EVENTS);
    }

    #[tokio::test(start_paused = true)]
    async fn participant_names_are_unique_even_after_the_vocabulary_cycles() {
        let rooms = test_rooms(EMPTY_TTL);
        let created = rooms.create().unwrap();
        let room = rooms.get(&created.room_id).unwrap();
        let host = room.join(Some(created.host_token)).await.unwrap();
        let mut names = std::collections::HashSet::new();
        names.insert(host.snapshots.borrow().participants[0].name.clone());

        for _ in 0..300 {
            let guest = room.join(None).await.unwrap();
            let joined = guest.snapshots.borrow().clone();
            let participant = joined.participants.last().unwrap();
            assert_eq!(participant.name.split_whitespace().count(), 2);
            assert!(names.insert(participant.name.clone()));
            assert_eq!(host.snapshots.borrow().participants, joined.participants);
            drop(guest);
        }
        settle().await;

        assert_eq!(host.snapshots.borrow().participants.len(), 1);
        assert_eq!(host.snapshots.borrow().revision, 0);
        assert_eq!(host.snapshots.borrow().events.len(), MAX_EVENTS);
    }

    #[test]
    fn random_name_permutations_cover_every_pair_without_repeats() {
        for (offset, stride) in [(0, 1), (231, 73), (255, 255)] {
            let names: std::collections::HashSet<_> = (1..=256)
                .map(|id| member_name(id, offset, stride))
                .collect();

            assert_eq!(names.len(), 256);
            assert!(
                names
                    .iter()
                    .all(|name| name.split_whitespace().count() == 2)
            );
            assert!(!names.contains(&member_name(257, offset, stride)));
        }

        assert_ne!(member_name(1, 0, 1), member_name(1, 231, 73));
    }

    #[test]
    fn wire_types_have_exact_protocol_shapes() {
        let action: Action =
            serde_json::from_str(r#"{"type":"set_video","video_id":"dQw4w9WgXcQ"}"#).unwrap();
        assert_eq!(action, video());
        let rate: Action =
            serde_json::from_str(r#"{"type":"set_rate","playback_rate":1.5}"#).unwrap();
        assert_eq!(rate, Action::SetRate { playback_rate: 1.5 });
        assert_eq!(
            serde_json::to_value(rate).unwrap(),
            serde_json::json!({
                "type": "set_rate", "playback_rate": 1.5,
            })
        );
        assert!(
            serde_json::from_str::<Action>(r#"{"type":"seek","position_secs":1,"extra":true}"#)
                .is_err()
        );
        assert_eq!(serde_json::to_value(Role::Host).unwrap(), "host");
        assert_eq!(serde_json::to_value(Role::Guest).unwrap(), "guest");
        assert_eq!(serde_json::to_value(Role::Moderator).unwrap(), "moderator");
        let social: SocialAction =
            serde_json::from_str(r#"{"type":"set_moderator","member_id":2,"enabled":true}"#)
                .unwrap();
        assert_eq!(
            social,
            SocialAction::SetModerator {
                member_id: 2,
                enabled: true
            }
        );
        assert!(
            serde_json::from_str::<SocialAction>(
                r#"{"type":"message","text":"hello","extra":true}"#,
            )
            .is_err()
        );
        let snapshot = Snapshot {
            incarnation: "random".into(),
            room_name: "The Sleepy Observatory".into(),
            revision: 0,
            media_revision: 0,
            video_id: None,
            playing: false,
            playback_rate: 1.0,
            position_secs: 0.0,
            anchor_ms: 0,
            members: 1,
            events: Vec::new(),
            participants: vec![Participant {
                id: 1,
                name: "Sleepy Waffle".into(),
                role: Role::Guest,
                avatar: "0123456789abcdef".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            serde_json::json!({
                "incarnation": "random", "room_name": "The Sleepy Observatory", "revision": 0, "media_revision": 0, "video_id": null,
                "playing": false, "playback_rate": 1.0, "position_secs": 0.0, "anchor_ms": 0, "members": 1,
                "participants": [{ "id": 1, "name": "Sleepy Waffle", "role": "guest", "avatar": "0123456789abcdef" }],
                "events": [],
            })
        );
        let created = CreatedRoom {
            room_id: "room".into(),
            host_token: "secret".into(),
        };
        assert_eq!(
            serde_json::to_value(created).unwrap(),
            serde_json::json!({
                "room_id": "room", "host_token": "secret",
            })
        );
    }
}
