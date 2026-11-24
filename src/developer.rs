//! Bounded, memory-only developer diagnostics and plain-data UI integration.
mod bridge;
mod discovery;

pub use bridge::DeveloperService;

#[derive(Clone, Debug)]
pub struct PairingRequest {
    pub id: String,
    pub session_id: String,
    pub workspace: String,
}
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_FRAME_BYTES: usize = 65536;
const MAX_LOG_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeveloperPreferences {
    pub enabled: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferenceSave {
    Durable,
    DurabilityUnconfirmed,
}
impl DeveloperPreferences {
    pub fn load_from(path: &Path) -> Result<Self, String> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(_) => return Err("Cannot read developer preferences".into()),
        };
        let mut bytes = Vec::new();
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read developer preferences")?;
        if bytes.len() > 4096 {
            return Err("Developer preferences exceed size limit; file preserved".into());
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| "Invalid developer preferences; file preserved".into())
    }
    pub fn save_to(&self, path: &Path) -> Result<PreferenceSave, String> {
        self.save_with_directory_sync(path, |parent| {
            #[cfg(unix)]
            File::open(parent)?.sync_all()?;
            #[cfg(not(unix))]
            let _ = parent;
            Ok(())
        })
    }
    fn save_with_directory_sync(
        &self,
        path: &Path,
        sync_directory: impl FnOnce(&Path) -> std::io::Result<()>,
    ) -> Result<PreferenceSave, String> {
        Self::load_from(path)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|_| "Cannot create developer preference directory")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| "Cannot create developer preference temporary file")?;
        let bytes = serde_json::to_vec(self).map_err(|_| "Cannot encode developer preferences")?;
        temp.write_all(&bytes)
            .and_then(|()| temp.as_file().sync_all())
            .map_err(|_| "Cannot write developer preferences")?;
        temp.persist(path)
            .map_err(|_| "Cannot replace developer preferences")?;
        Ok(if sync_directory(parent).is_ok() {
            PreferenceSave::Durable
        } else {
            PreferenceSave::DurabilityUnconfirmed
        })
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(9_007_199_254_740_991) as u64
}
pub(crate) fn random_id() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Cannot create developer capability")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
macro_rules! wire_enum { ($name:ident { $($variant:ident),* }) => {
    #[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all="snake_case")]
    pub enum $name { $($variant),* }
}; }
wire_enum!(Level {
    Debug,
    Info,
    Warn,
    Error
});
wire_enum!(Target {
    App,
    Open,
    List,
    Decode,
    Navigation,
    Preferences,
    Release
});
wire_enum!(EventCode {
    Started,
    Completed,
    Failed,
    Changed
});
wire_enum!(Field {
    Count,
    PageIndex,
    Success,
    ErrorCode,
    Width,
    Height
});
wire_enum!(UiMode {
    Reading,
    Browsing,
    Preferences
});
wire_enum!(ReadingMode {
    Single,
    Spread,
    Continuous
});
wire_enum!(FitMode {
    Window,
    Width,
    Height,
    Original,
    Custom
});
wire_enum!(TargetKind { Element, Region });
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Number(u64),
    Bool(bool),
}
#[derive(Clone, Debug)]
pub struct DiagnosticEvent {
    pub level: Level,
    pub target: Target,
    pub code: EventCode,
    pub operation_id: u64,
    pub duration_ms: u64,
    pub fields: Vec<(Field, Scalar)>,
}
#[derive(Clone, Debug, Serialize)]
pub struct LogEvent {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub level: Level,
    pub target: Target,
    pub code: EventCode,
    pub operation_id: u64,
    pub duration_ms: u64,
    pub fields: BTreeMap<String, Scalar>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub capture_id: String,
    pub sequence: u64,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogQuery {
    pub after: Option<Cursor>,
    pub level: Option<Level>,
    pub target: Option<Target>,
    pub contains: Option<String>,
    pub limit: Option<usize>,
}
#[derive(Clone, Debug, Serialize)]
pub struct LogPage {
    pub capture_id: String,
    pub events: Vec<LogEvent>,
    pub after: Cursor,
    pub oldest_sequence: u64,
    pub dropped: u64,
    pub truncated: bool,
}
struct Ring {
    events: VecDeque<(LogEvent, usize)>,
    bytes: usize,
    next: u64,
    dropped: u64,
    active: bool,
}
#[derive(Clone)]
pub struct LogStore {
    capture_id: String,
    ring: Arc<Mutex<Ring>>,
}
impl LogStore {
    fn new(capture_id: String) -> Self {
        Self {
            capture_id,
            ring: Arc::new(Mutex::new(Ring {
                events: VecDeque::new(),
                bytes: 0,
                next: 1,
                dropped: 0,
                active: true,
            })),
        }
    }
    pub fn record(&self, event: DiagnosticEvent) {
        if event.fields.len() > 16
            || event.operation_id > 9_007_199_254_740_991
            || event.duration_ms > 9_007_199_254_740_991
            || event
                .fields
                .iter()
                .any(|(_, v)| matches!(v,Scalar::Number(n) if *n > 9_007_199_254_740_991))
        {
            return;
        }
        let fields = event
            .fields
            .into_iter()
            .map(|(k, v)| {
                (
                    serde_json::to_value(k)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    v,
                )
            })
            .collect();
        let mut ring = self.ring.lock().unwrap();
        if !ring.active {
            return;
        }
        let entry = LogEvent {
            sequence: ring.next,
            timestamp_ms: now_ms(),
            level: event.level,
            target: event.target,
            code: event.code,
            operation_id: event.operation_id,
            duration_ms: event.duration_ms,
            fields,
        };
        let size = serde_json::to_vec(&entry).unwrap().len();
        if size > 8192 {
            return;
        }
        ring.next += 1;
        while ring.events.len() >= 5000 || ring.bytes + size > MAX_LOG_BYTES {
            if let Some((_, bytes)) = ring.events.pop_front() {
                ring.bytes -= bytes;
                ring.dropped += 1;
            }
        }
        ring.bytes += size;
        ring.events.push_back((entry, size));
    }
    pub fn query(&self, query: &LogQuery) -> Result<LogPage, BridgeError> {
        let limit = query.limit.unwrap_or(100);
        if !(1..=200).contains(&limit) || query.contains.as_ref().is_some_and(|s| s.len() > 256) {
            return Err(BridgeError::InvalidRequest);
        }
        if query
            .after
            .as_ref()
            .is_some_and(|c| c.capture_id != self.capture_id)
        {
            return Err(BridgeError::CaptureChanged);
        }
        let ring = self.ring.lock().unwrap();
        let mut page = LogPage {
            capture_id: self.capture_id.clone(),
            events: Vec::new(),
            after: query.after.clone().unwrap_or(Cursor {
                capture_id: self.capture_id.clone(),
                sequence: 0,
            }),
            oldest_sequence: ring.events.front().map_or(ring.next, |(e, _)| e.sequence),
            dropped: ring.dropped,
            truncated: false,
        };
        let mut page_bytes = serde_json::to_vec(&page).unwrap().len();
        for (event, event_bytes) in &ring.events {
            if event.sequence <= page.after.sequence
                || query.level.is_some_and(|l| l != event.level)
                || query.target.is_some_and(|t| t != event.target)
            {
                continue;
            }
            if query
                .contains
                .as_ref()
                .is_some_and(|s| !serde_json::to_string(event).unwrap().contains(s))
            {
                continue;
            }
            if page.events.len() == limit {
                page.truncated = true;
                break;
            }
            let candidate_bytes = page_bytes
                + event_bytes
                + usize::from(!page.events.is_empty())
                + event.sequence.to_string().len()
                - page.after.sequence.to_string().len();
            // Leave headroom for envelope, request ID and truncation metadata.
            if candidate_bytes > 60_000 {
                page.truncated = true;
                break;
            }
            page.events.push(event.clone());
            page.after.sequence = event.sequence;
            page_bytes = candidate_bytes;
        }
        while serde_json::to_vec(&page).unwrap().len() > 60_000 {
            page.events
                .pop()
                .expect("log page metadata fits the byte limit");
            page.after.sequence = page.events.last().map_or_else(
                || query.after.as_ref().map_or(0, |cursor| cursor.sequence),
                |event| event.sequence,
            );
            page.truncated = true;
        }
        Ok(page)
    }
    fn clear(&self) {
        let mut r = self.ring.lock().unwrap();
        r.active = false;
        r.events.clear();
        r.bytes = 0;
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeError {
    Disconnected,
    Timeout,
    ProtocolMismatch,
    Canceled,
    Busy,
    InvalidRequest,
    CaptureChanged,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Disabled,
    WaitingForPi,
    Connected,
    Failed,
}
#[derive(Clone, Debug)]
pub struct ServiceStatus {
    /// Local connection identity. This value is not part of the wire protocol.
    pub generation: u64,
    pub state: ConnectionState,
    pub session_id: Option<String>,
    pub workspace: Option<String>,
    pub capture_id: String,
}
#[derive(Clone, Debug)]
pub enum ServiceEvent {
    Connected,
    Disconnected,
    FeedbackHandedOff(String),
    FeedbackFailed(String),
    DeliveryUnknown(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackTarget {
    pub id: String,
    pub role: String,
    pub label: String,
    pub source: String,
    pub window_id: String,
    pub view_revision: u64,
    pub target_kind: TargetKind,
    pub bounds: Bounds,
    pub click: Point,
    pub scale_factor: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackState {
    pub ui_mode: UiMode,
    pub reading_mode: ReadingMode,
    pub page_index: Option<u64>,
    pub fit_mode: FitMode,
    pub app_version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feedback {
    pub feedback_id: String,
    pub capture_id: String,
    pub session_id: String,
    pub captured_at_ms: u64,
    pub target: FeedbackTarget,
    pub state: FeedbackState,
    pub text: String,
}
impl Feedback {
    pub fn new_id() -> Result<String, String> {
        random_id()
    }
    pub(crate) fn valid(&self) -> bool {
        let t = &self.target;
        let number = |v: f64| v.is_finite() && v.abs() <= 1e7;
        valid_hex_id(&self.feedback_id)
            && valid_hex_id(&self.capture_id)
            && !self.session_id.is_empty()
            && self.session_id.len() <= 512
            && valid_id(&t.id)
            && valid_id(&t.role)
            && valid_id(&t.window_id)
            && !t.label.is_empty()
            && t.label.len() <= 256
            && !t.source.is_empty()
            && t.source.len() <= 256
            && t.view_revision <= 9_007_199_254_740_991
            && self
                .state
                .page_index
                .is_none_or(|v| v <= 9_007_199_254_740_991)
            && [
                t.bounds.x,
                t.bounds.y,
                t.bounds.width,
                t.bounds.height,
                t.click.x,
                t.click.y,
            ]
            .into_iter()
            .all(number)
            && t.bounds.width > 0.
            && t.bounds.height > 0.
            && (0.1..=16.).contains(&t.scale_factor)
            && self.captured_at_ms <= 9_007_199_254_740_991
            && !self.state.app_version.is_empty()
            && self.state.app_version.len() <= 64
            && !self.text.trim().is_empty()
            && self.text.len() <= 8192
    }
}
pub(crate) fn valid_hex_id(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event() -> DiagnosticEvent {
        DiagnosticEvent {
            level: Level::Info,
            target: Target::Open,
            code: EventCode::Completed,
            operation_id: 1,
            duration_ms: 2,
            fields: vec![(Field::Count, Scalar::Number(3))],
        }
    }
    #[test]
    fn preferences_preserve_invalid() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("developer-mode.json");
        assert!(!DeveloperPreferences::load_from(&p).unwrap().enabled);
        for bytes in [
            b"null".to_vec(),
            b"{}".to_vec(),
            b"{\"enabled\":null}".to_vec(),
            b"{\"enabled\":true,\"extra\":1}".to_vec(),
            vec![b' '; 4097],
        ] {
            fs::write(&p, &bytes).unwrap();
            assert!(DeveloperPreferences { enabled: true }.save_to(&p).is_err());
            assert_eq!(fs::read(&p).unwrap(), bytes);
        }
        fs::remove_file(&p).unwrap();
        DeveloperPreferences { enabled: true }.save_to(&p).unwrap();
        assert!(DeveloperPreferences::load_from(&p).unwrap().enabled);
        DeveloperPreferences { enabled: false }.save_to(&p).unwrap();
        assert!(!DeveloperPreferences::load_from(&p).unwrap().enabled);
    }
    #[test]
    fn preference_replacement_reports_directory_sync_failure_as_applied() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("developer-mode.json");
        for enabled in [true, false] {
            let outcome = DeveloperPreferences { enabled }
                .save_with_directory_sync(&path, |_| Err(std::io::Error::other("sync failed")))
                .unwrap();
            assert_eq!(outcome, PreferenceSave::DurabilityUnconfirmed);
            assert_eq!(
                DeveloperPreferences::load_from(&path).unwrap().enabled,
                enabled
            );
        }
        fs::write(&path, b"invalid").unwrap();
        assert!(
            DeveloperPreferences { enabled: true }
                .save_with_directory_sync(&path, |_| panic!("must not sync an unapplied save"))
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"invalid");
    }
    #[test]
    fn ring_eviction_filters_and_capture() {
        let logs = LogStore::new("a".into());
        for _ in 0..5002 {
            logs.record(event());
        }
        let p = logs.query(&LogQuery::default()).unwrap();
        assert_eq!(p.oldest_sequence, 3);
        assert_eq!(p.dropped, 2);
        assert_eq!(p.events.len(), 100);
        assert!(p.truncated);
        assert_eq!(p.after.sequence, 102);
        assert_eq!(
            logs.query(&LogQuery {
                after: Some(Cursor {
                    capture_id: "old".into(),
                    sequence: 0
                }),
                ..Default::default()
            })
            .unwrap_err(),
            BridgeError::CaptureChanged
        );
        assert!(
            logs.query(&LogQuery {
                contains: Some("[.*]".into()),
                ..Default::default()
            })
            .unwrap()
            .events
            .is_empty()
        );
        logs.clear();
        logs.record(event());
        assert!(logs.query(&LogQuery::default()).unwrap().events.is_empty());
    }
    #[test]
    fn reviewed_filters_and_query_limits() {
        let logs = LogStore::new("a".repeat(64));
        logs.record(event());
        let mut other = event();
        other.level = Level::Error;
        other.target = Target::Decode;
        other.code = EventCode::Failed;
        logs.record(other);
        let q = LogQuery {
            level: Some(Level::Error),
            target: Some(Target::Decode),
            contains: Some("failed".into()),
            ..Default::default()
        };
        let page = logs.query(&q).unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].sequence, 2);
        assert!(
            logs.query(&LogQuery {
                level: Some(Level::Warn),
                ..Default::default()
            })
            .unwrap()
            .events
            .is_empty()
        );
        for limit in [0, 201] {
            assert_eq!(
                logs.query(&LogQuery {
                    limit: Some(limit),
                    ..Default::default()
                })
                .unwrap_err(),
                BridgeError::InvalidRequest
            );
        }
        assert_eq!(
            logs.query(&LogQuery {
                contains: Some("x".repeat(257)),
                ..Default::default()
            })
            .unwrap_err(),
            BridgeError::InvalidRequest
        );
        let mut invalid = event();
        invalid.fields = vec![(Field::Count, Scalar::Number(u64::MAX))];
        logs.record(invalid);
        assert_eq!(logs.ring.lock().unwrap().next, 3);
    }
    #[test]
    fn byte_pagination_does_not_skip_events() {
        let logs = LogStore::new("a".into());
        for _ in 0..300 {
            let mut e = event();
            e.operation_id = 9_007_199_254_740_991;
            e.duration_ms = 9_007_199_254_740_991;
            e.fields = vec![
                Field::Count,
                Field::PageIndex,
                Field::Success,
                Field::ErrorCode,
                Field::Width,
                Field::Height,
            ]
            .into_iter()
            .map(|f| (f, Scalar::Number(9_007_199_254_740_991)))
            .collect();
            logs.record(e);
        }
        let first = logs
            .query(&LogQuery {
                limit: Some(200),
                ..Default::default()
            })
            .unwrap();
        assert!(first.truncated);
        assert!(first.events.len() < 200);
        assert!(serde_json::to_vec(&first).unwrap().len() <= 60_000);
        assert_eq!(first.after.sequence, first.events.last().unwrap().sequence);
        let next = logs
            .query(&LogQuery {
                after: Some(first.after.clone()),
                limit: Some(200),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(next.events[0].sequence, first.after.sequence + 1);
        let ring = logs.ring.lock().unwrap();
        assert!(ring.bytes <= MAX_LOG_BYTES);
        assert_eq!(
            ring.bytes,
            ring.events.iter().map(|(_, s)| s).sum::<usize>()
        );
    }
    #[test]
    fn concurrent_records() {
        let logs = LogStore::new("a".into());
        std::thread::scope(|s| {
            for _ in 0..4 {
                let l = logs.clone();
                s.spawn(move || {
                    for _ in 0..500 {
                        l.record(event())
                    }
                });
            }
        });
        assert_eq!(logs.ring.lock().unwrap().next, 2001);
    }
}
