//! Single bounded nonblocking socket worker. No UI values cross this boundary.
use super::*;
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const IO_DEADLINE: Duration = Duration::from_secs(2);
struct Shared {
    status: ServiceStatus,
    token: Option<(String, Instant)>,
    generation: u64,
    ui_mode: UiMode,
    completions: VecDeque<ServiceEvent>,
    outstanding: usize,
}
pub struct DeveloperService {
    shared: Arc<Mutex<Shared>>,
    logs: LogStore,
    stop: Arc<AtomicBool>,
    outbound: SyncSender<(u64, Feedback)>,
    events: Receiver<ServiceEvent>,
    worker: Option<JoinHandle<()>>,
    port: u16,
}
impl DeveloperService {
    pub fn start() -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "Cannot start developer listener")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Cannot configure developer listener")?;
        let port = listener
            .local_addr()
            .map_err(|_| "Cannot read developer port")?
            .port();
        let capture_id = random_id()?;
        let logs = LogStore::new(capture_id.clone());
        let shared = Arc::new(Mutex::new(Shared {
            status: ServiceStatus {
                generation: 0,
                state: ConnectionState::WaitingForPi,
                session_id: None,
                workspace: None,
                capture_id,
            },
            token: Some((random_id()?, Instant::now())),
            generation: 0,
            ui_mode: UiMode::Reading,
            completions: VecDeque::new(),
            outstanding: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (outbound, rx) = mpsc::sync_channel(32);
        let (tx, events) = mpsc::sync_channel(32);
        let (s, l, st) = (shared.clone(), logs.clone(), stop.clone());
        let worker = thread::Builder::new()
            .name("developer-bridge".into())
            .spawn(move || serve(listener, s, l, st, rx, tx))
            .map_err(|_| "Cannot start developer worker")?;
        Ok(Self {
            shared,
            logs,
            stop,
            outbound,
            events,
            worker: Some(worker),
            port,
        })
    }
    pub fn status(&self) -> ServiceStatus {
        let shared = self.shared.lock().unwrap();
        let mut status = shared.status.clone();
        status.generation = shared.generation;
        status
    }
    pub fn logs(&self) -> LogStore {
        self.logs.clone()
    }
    pub fn set_ui_mode(&self, mode: UiMode) {
        self.shared.lock().unwrap().ui_mode = mode;
    }
    pub fn connection_command(&self) -> Result<String, String> {
        let mut s = self.shared.lock().unwrap();
        if s.status.state == ConnectionState::Connected {
            return Err("Disconnect Pi before copying a new connection command".into());
        }
        let token = random_id()?;
        s.token = Some((token.clone(), Instant::now()));
        Ok(format!("/mangameeya-connect {} {}", self.port, token))
    }
    pub fn disconnect(&self) {
        let mut s = self.shared.lock().unwrap();
        s.generation += 1;
        rotate(&mut s);
    }
    pub fn try_send_feedback(&self, feedback: Feedback) -> Result<(), BridgeError> {
        if !feedback.valid() {
            return Err(BridgeError::InvalidRequest);
        }
        let mut s = self.shared.lock().unwrap();
        if s.status.state != ConnectionState::Connected
            || s.status.session_id.as_ref() != Some(&feedback.session_id)
            || s.status.capture_id != feedback.capture_id
        {
            return Err(BridgeError::Disconnected);
        }
        if s.outstanding >= 32 {
            return Err(BridgeError::Busy);
        }
        self.outbound
            .try_send((s.generation, feedback))
            .map_err(|e| match e {
                mpsc::TrySendError::Full(_) => BridgeError::Busy,
                _ => BridgeError::Disconnected,
            })?;
        s.outstanding += 1;
        Ok(())
    }
    pub fn try_recv_event(&self) -> Option<ServiceEvent> {
        let mut s = self.shared.lock().unwrap();
        if let Some(e) = s.completions.pop_front() {
            s.outstanding -= 1;
            return Some(e);
        }
        self.events.try_recv().ok()
    }
}
impl Drop for DeveloperService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        self.logs.clear();
    }
}
fn rotate(s: &mut Shared) {
    s.status.session_id = None;
    s.status.workspace = None;
    s.token = random_id().ok().map(|t| (t, Instant::now()));
    s.status.state = if s.token.is_some() {
        ConnectionState::WaitingForPi
    } else {
        ConnectionState::Failed
    };
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Hello {
        v: u8,
        id: String,
        token: String,
        session_id: String,
        workspace: String,
    },
    Status {
        v: u8,
        id: String,
    },
    Logs {
        v: u8,
        id: String,
        after: Option<Cursor>,
        level: Option<Level>,
        target: Option<Target>,
        contains: Option<String>,
        limit: Option<usize>,
    },
    Ack {
        v: u8,
        id: String,
        feedback_id: String,
        outcome: Outcome,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    HandedOff,
    Failed,
}
impl Request {
    fn identity(&self) -> (u8, &str) {
        match self {
            Self::Hello { v, id, .. }
            | Self::Status { v, id }
            | Self::Logs { v, id, .. }
            | Self::Ack { v, id, .. } => (*v, id),
        }
    }
}
struct PendingWrite {
    bytes: Vec<u8>,
    offset: usize,
    started: Instant,
}
struct Peer {
    socket: TcpStream,
    header: [u8; 4],
    header_read: usize,
    body: Vec<u8>,
    body_read: usize,
    frame_started: Option<Instant>,
    accepted: Instant,
    paired: bool,
    generation: u64,
    writes: VecDeque<PendingWrite>,
    acks: BTreeMap<String, Instant>,
}
impl Peer {
    fn new(socket: TcpStream) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            header: [0; 4],
            header_read: 0,
            body: Vec::new(),
            body_read: 0,
            frame_started: None,
            accepted: Instant::now(),
            paired: false,
            generation: 0,
            writes: VecDeque::new(),
            acks: BTreeMap::new(),
        })
    }
    fn enqueue(&mut self, value: Value) -> bool {
        if self.writes.len() >= 32 {
            return false;
        }
        let Ok(body) = serde_json::to_vec(&value) else {
            return false;
        };
        if body.len() > MAX_FRAME_BYTES {
            return false;
        }
        let mut bytes = Vec::with_capacity(body.len() + 4);
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend(body);
        self.writes.push_back(PendingWrite {
            bytes,
            offset: 0,
            started: Instant::now(),
        });
        true
    }
    fn read_frame(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self
            .frame_started
            .is_some_and(|t| t.elapsed() > IO_DEADLINE)
        {
            return Err(io::ErrorKind::TimedOut.into());
        }
        if self.header_read < 4 {
            match self.socket.read(&mut self.header[self.header_read..]) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => {
                    self.frame_started.get_or_insert_with(Instant::now);
                    self.header_read += n;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(e) => return Err(e),
            }
            if self.header_read < 4 {
                return Ok(None);
            }
            let len = u32::from_be_bytes(self.header) as usize;
            if len == 0 || len > MAX_FRAME_BYTES {
                return Err(io::ErrorKind::InvalidData.into());
            }
            self.body = vec![0; len];
        }
        match self.socket.read(&mut self.body[self.body_read..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => self.body_read += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(e) => return Err(e),
        }
        if self.body_read != self.body.len() {
            return Ok(None);
        }
        self.header_read = 0;
        self.body_read = 0;
        self.frame_started = None;
        Ok(Some(std::mem::take(&mut self.body)))
    }
    fn flush_current(&mut self, shared: &Mutex<Shared>) -> io::Result<()> {
        // Disconnect must not advance the generation between this check and write.
        // The socket is nonblocking, so the lock does not wait for the reader.
        let s = shared.lock().unwrap();
        if self.paired && self.generation != s.generation {
            self.writes.clear();
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
        self.flush()
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(w) = self.writes.front_mut() {
            if w.started.elapsed() > IO_DEADLINE {
                return Err(io::ErrorKind::TimedOut.into());
            }
            match self.socket.write(&w.bytes[w.offset..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => w.offset += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            }
            if w.offset == w.bytes.len() {
                self.writes.pop_front();
            }
        }
        Ok(())
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}
fn error(peer: &mut Peer, id: &str, code: BridgeError) -> bool {
    peer.enqueue(json!({"v":1,"id":id,"type":"error","code":code}))
}
fn handle(
    peer: &mut Peer,
    bytes: &[u8],
    shared: &Mutex<Shared>,
    logs: &LogStore,
    events: &SyncSender<ServiceEvent>,
    started: u64,
) -> bool {
    let Ok(request) = serde_json::from_slice::<Request>(bytes) else {
        return false;
    };
    let (v, id) = request.identity();
    if !valid_id(id) {
        return false;
    }
    if v != PROTOCOL_VERSION {
        let _ = error(peer, id, BridgeError::ProtocolMismatch);
        let _ = peer.flush_current(shared);
        return false;
    }
    if !peer.paired {
        let Request::Hello {
            id,
            token,
            session_id,
            workspace,
            ..
        } = request
        else {
            return false;
        };
        if token.len() != 64
            || session_id.is_empty()
            || session_id.len() > 512
            || workspace.is_empty()
            || workspace.len() > 2048
        {
            return false;
        }
        let mut s = shared.lock().unwrap();
        if s.status.state != ConnectionState::WaitingForPi
            || !s.token.as_ref().is_some_and(|(t, issued)| {
                issued.elapsed() < Duration::from_secs(300)
                    && constant_equal(t.as_bytes(), token.as_bytes())
            })
        {
            return false;
        }
        s.token = None;
        s.status.state = ConnectionState::Connected;
        s.status.session_id = Some(session_id);
        s.status.workspace = Some(workspace);
        peer.paired = true;
        peer.generation = s.generation;
        let _ = events.try_send(ServiceEvent::Connected);
        return peer
            .enqueue(json!({"v":1,"id":id,"type":"hello_ok","capture_id":s.status.capture_id}));
    }
    if peer.writes.len() >= 31 && !matches!(request, Request::Ack { .. }) {
        return error(peer, id, BridgeError::Busy);
    }
    match request {
        Request::Status { id, .. } => {
            let s = shared.lock().unwrap();
            peer.enqueue(status_response(&id, &s, started))
        }
        Request::Logs {
            id,
            after,
            level,
            target,
            contains,
            limit,
            ..
        } => match logs.query(&LogQuery {
            after,
            level,
            target,
            contains,
            limit,
        }) {
            Ok(page) => {
                let mut value = serde_json::to_value(page).unwrap();
                let o = value.as_object_mut().unwrap();
                o.insert("v".into(), json!(1));
                o.insert("id".into(), json!(id));
                o.insert("type".into(), json!("logs_result"));
                peer.enqueue(value)
            }
            Err(e) => error(peer, &id, e),
        },
        Request::Ack {
            id,
            feedback_id,
            outcome,
            ..
        } => {
            if id != feedback_id || !valid_id(&feedback_id) {
                return false;
            }
            if peer.acks.remove(&feedback_id).is_some() {
                let event = match outcome {
                    Outcome::HandedOff => ServiceEvent::FeedbackHandedOff(feedback_id),
                    Outcome::Failed => ServiceEvent::FeedbackFailed(feedback_id),
                };
                complete(shared, event);
            }
            true
        }
        _ => false,
    }
}
fn status_response(id: &str, s: &Shared, started: u64) -> Value {
    json!({"v":1,"id":id,"type":"status_result","status":{"app_version":env!("CARGO_PKG_VERSION"),"protocol_version":PROTOCOL_VERSION,"instance_id":s.status.capture_id,"capture_id":s.status.capture_id,"session_id":s.status.session_id,"capture_started_ms":started,"ui_mode":s.ui_mode}})
}
fn complete(shared: &Mutex<Shared>, event: ServiceEvent) {
    let mut s = shared.lock().unwrap();
    debug_assert!(s.completions.len() < 32);
    s.completions.push_back(event);
}
fn constant_equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}
fn serve(
    listener: TcpListener,
    shared: Arc<Mutex<Shared>>,
    logs: LogStore,
    stop: Arc<AtomicBool>,
    outbound: Receiver<(u64, Feedback)>,
    events: SyncSender<ServiceEvent>,
) {
    let started = now_ms();
    let mut peers: Vec<Peer> = Vec::new();
    while !stop.load(Ordering::Acquire) {
        // Accept a bounded batch; a continuous connection flood cannot starve I/O.
        for _ in 0..8 {
            match listener.accept() {
                Ok((socket, _)) => {
                    if peers.iter().filter(|p| !p.paired).count() < 4
                        && let Ok(p) = Peer::new(socket)
                    {
                        peers.push(p);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        {
            let mut s = shared.lock().unwrap();
            if s.token
                .as_ref()
                .is_some_and(|(_, t)| t.elapsed() >= Duration::from_secs(300))
            {
                rotate(&mut s);
            }
        }
        for _ in 0..32 {
            let Ok((submitted_generation, feedback)) = outbound.try_recv() else {
                break;
            };
            let id = feedback.feedback_id.clone();
            let generation = shared.lock().unwrap().generation;
            if let Some(peer) = peers.iter_mut().find(|p| {
                p.paired && p.generation == generation && p.generation == submitted_generation
            }) {
                // Keep pending acknowledgments and outbound allocations bounded.
                if peer.acks.len() < 32
                    && !peer.acks.contains_key(&id)
                    && peer.enqueue(json!({"v":1,"id":id,"type":"feedback","feedback":feedback}))
                {
                    peer.acks.insert(id, Instant::now());
                } else {
                    complete(&shared, ServiceEvent::FeedbackFailed(id));
                }
            } else {
                complete(&shared, ServiceEvent::FeedbackFailed(id));
            }
        }
        let mut i = 0;
        while i < peers.len() {
            let peer = &mut peers[i];
            let generation = shared.lock().unwrap().generation;
            let mut alive = if peer.paired {
                peer.generation == generation
            } else {
                peer.accepted.elapsed() < IO_DEADLINE
            };
            if alive {
                match peer.read_frame() {
                    Ok(Some(bytes)) => {
                        alive = handle(peer, &bytes, &shared, &logs, &events, started)
                    }
                    Ok(None) => {}
                    Err(_) => alive = false,
                }
            }
            if alive {
                alive = peer.flush_current(&shared).is_ok();
            }
            let expired: Vec<_> = peer
                .acks
                .iter()
                .filter(|(_, t)| t.elapsed() > Duration::from_secs(5))
                .map(|(id, _)| id.clone())
                .collect();
            for id in expired {
                peer.acks.remove(&id);
                complete(&shared, ServiceEvent::DeliveryUnknown(id));
            }
            if !alive {
                let peer = peers.swap_remove(i);
                if peer.paired {
                    let mut s = shared.lock().unwrap();
                    if s.generation == peer.generation {
                        s.generation += 1;
                        rotate(&mut s);
                    }
                    drop(s);
                    for id in peer.acks.keys() {
                        complete(&shared, ServiceEvent::DeliveryUnknown(id.clone()));
                    }
                    let _ = events.try_send(ServiceEvent::Disconnected);
                }
            } else {
                i += 1;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn send(socket: &mut TcpStream, value: Value) {
        let b = serde_json::to_vec(&value).unwrap();
        socket.write_all(&(b.len() as u32).to_be_bytes()).unwrap();
        for chunk in b.chunks(3) {
            socket.write_all(chunk).unwrap();
        }
    }
    fn assert_closed(socket: &mut TcpStream) {
        match socket.read(&mut [0; 1]) {
            Ok(0) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::BrokenPipe
                ) => {}
            other => panic!("expected closed socket, got {other:?}"),
        }
    }
    fn receive(socket: &mut TcpStream) -> Value {
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut h = [0; 4];
        socket.read_exact(&mut h).unwrap();
        let mut b = vec![0; u32::from_be_bytes(h) as usize];
        socket.read_exact(&mut b).unwrap();
        serde_json::from_slice(&b).unwrap()
    }
    fn connect(service: &DeveloperService) -> TcpStream {
        let cmd = service.connection_command().unwrap();
        let token = cmd.split_whitespace().last().unwrap();
        let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, service.port)).unwrap();
        send(
            &mut socket,
            json!({"v":1,"id":"h","type":"hello","token":token,"session_id":"test","workspace":"fixture"}),
        );
        assert_eq!(receive(&mut socket)["type"], "hello_ok");
        socket
    }
    fn feedback(service: &DeveloperService, id: &str) -> Feedback {
        Feedback {
            feedback_id: id.into(),
            capture_id: service.status().capture_id,
            session_id: "test".into(),
            captured_at_ms: now_ms(),
            target: FeedbackTarget {
                id: "reader.next".into(),
                role: "button".into(),
                label: "Next page".into(),
                source: "src/ui.rs::render".into(),
                window_id: "reader".into(),
                view_revision: 1,
                target_kind: TargetKind::Element,
                bounds: Bounds {
                    x: 0.,
                    y: 0.,
                    width: 10.,
                    height: 10.,
                },
                click: Point { x: 2., y: 2. },
                scale_factor: 1.,
            },
            state: FeedbackState {
                ui_mode: UiMode::Reading,
                reading_mode: ReadingMode::Single,
                page_index: Some(0),
                fit_mode: FitMode::Window,
                app_version: "0.9.0".into(),
            },
            text: "Assess the control".into(),
        }
    }
    fn completion(service: &DeveloperService) -> ServiceEvent {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(e) = service.try_recv_event()
                && !matches!(e, ServiceEvent::Connected | ServiceEvent::Disconnected)
            {
                return e;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn native_wire_contract_fixture() {
        let capture = "a".repeat(64);
        let service = DeveloperService::start().unwrap();
        let mut item = feedback(&service, &"b".repeat(64));
        item.capture_id = capture.clone();
        item.captured_at_ms = 1000;
        let event = LogEvent {
            sequence: 1,
            timestamp_ms: 1000,
            level: Level::Info,
            target: Target::Open,
            code: EventCode::Completed,
            operation_id: 7,
            duration_ms: 12,
            fields: BTreeMap::from([
                ("count".into(), Scalar::Number(2)),
                ("success".into(), Scalar::Bool(true)),
            ]),
        };
        let page = LogPage {
            capture_id: capture.clone(),
            events: vec![event],
            after: Cursor {
                capture_id: capture.clone(),
                sequence: 1,
            },
            oldest_sequence: 1,
            dropped: 0,
            truncated: false,
        };
        let mut logs = serde_json::to_value(page).unwrap();
        let map = logs.as_object_mut().unwrap();
        map.insert("v".into(), json!(1));
        map.insert("id".into(), json!("logs-1"));
        map.insert("type".into(), json!("logs_result"));
        let status = {
            let mut s = service.shared.lock().unwrap();
            s.status.capture_id = capture.clone();
            s.status.session_id = Some("test".into());
            status_response("status-1", &s, 1000)
        };
        let native = json!([
            {"v":1,"id":"hello-1","type":"hello_ok","capture_id":capture},
            status,
            logs,
            {"v":1,"id":item.feedback_id,"type":"feedback","feedback":item},
            {"v":1,"id":"error-1","type":"error","code":BridgeError::CaptureChanged}
        ]);
        let fixture: Value = serde_json::from_str(include_str!(
            "../../.pi/extensions/mangameeya-dev/fixtures/native-v1.json"
        ))
        .unwrap();
        assert_eq!(native, fixture);
    }
    #[test]
    fn feedback_ack_unknown_and_queue_reservation() {
        let service = DeveloperService::start().unwrap();
        let mut socket = connect(&service);
        service
            .try_send_feedback(feedback(&service, &"a".repeat(64)))
            .unwrap();
        assert_eq!(
            receive(&mut socket)["feedback"]["target"]["id"],
            "reader.next"
        );
        send(
            &mut socket,
            json!({"v":1,"id":"a".repeat(64),"type":"ack","feedback_id":"a".repeat(64),"outcome":"handed_off"}),
        );
        assert!(
            matches!(completion(&service),ServiceEvent::FeedbackHandedOff(id) if id=="a".repeat(64))
        );
        service
            .try_send_feedback(feedback(&service, &"c".repeat(64)))
            .unwrap();
        receive(&mut socket);
        send(
            &mut socket,
            json!({"v":1,"id":"c".repeat(64),"type":"ack","feedback_id":"c".repeat(64),"outcome":"failed"}),
        );
        assert!(
            matches!(completion(&service),ServiceEvent::FeedbackFailed(id) if id=="c".repeat(64))
        );
        service
            .try_send_feedback(feedback(&service, &"b".repeat(64)))
            .unwrap();
        receive(&mut socket);
        drop(socket);
        assert!(
            matches!(completion(&service),ServiceEvent::DeliveryUnknown(id) if id=="b".repeat(64))
        );
        let _socket = connect(&service);
        for i in 0..32 {
            service
                .try_send_feedback(feedback(&service, &format!("{i:064x}")))
                .unwrap();
        }
        assert_eq!(
            service.try_send_feedback(feedback(&service, &"f".repeat(64))),
            Err(BridgeError::Busy)
        );
        service.disconnect();
        for _ in 0..32 {
            assert!(matches!(
                completion(&service),
                ServiceEvent::DeliveryUnknown(_) | ServiceEvent::FeedbackFailed(_)
            ));
        }
    }
    #[test]
    fn expiry_version_replay_and_strict_operations() {
        let service = DeveloperService::start().unwrap();
        let cmd = service.connection_command().unwrap();
        let old = cmd.split_whitespace().last().unwrap().to_string();
        service.shared.lock().unwrap().token.as_mut().unwrap().1 =
            Instant::now() - Duration::from_secs(301);
        let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, service.port)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        send(
            &mut socket,
            json!({"v":1,"id":"h","type":"hello","token":old,"session_id":"test","workspace":"fixture"}),
        );
        assert_closed(&mut socket);
        let mut socket = connect(&service);
        send(&mut socket, json!({"v":2,"id":"s","type":"status"}));
        assert_eq!(receive(&mut socket)["code"], "protocol_mismatch");
        drop(socket);
        let deadline = Instant::now() + Duration::from_secs(3);
        while service.status().state != ConnectionState::WaitingForPi {
            assert!(
                Instant::now() < deadline,
                "disconnected peer was not retired"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let mut socket = connect(&service);
        send(
            &mut socket,
            json!({"v":1,"id":"x","type":"status","unexpected":true}),
        );
        assert_closed(&mut socket);
    }
    #[test]
    fn disconnect_drops_queued_and_partial_diagnostic_writes() {
        let service = DeveloperService::start().unwrap();
        let _paired = connect(&service);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let (accepted, _) = listener.accept().unwrap();
        let mut peer = Peer::new(accepted).unwrap();
        peer.paired = true;
        peer.generation = service.status().generation;
        let (events, _receiver) = mpsc::sync_channel(32);
        let queue = |peer: &mut Peer, request: Value| {
            assert!(handle(
                peer,
                &serde_json::to_vec(&request).unwrap(),
                &service.shared,
                &service.logs,
                &events,
                1000,
            ));
        };
        queue(&mut peer, json!({"v":1,"id":"s","type":"status"}));
        while !peer.writes.is_empty() {
            peer.flush_current(&service.shared).unwrap();
        }
        assert_eq!(receive(&mut client)["status"]["session_id"], "test");

        queue(&mut peer, json!({"v":1,"id":"s2","type":"status"}));
        queue(&mut peer, json!({"v":1,"id":"l","type":"logs"}));
        assert!(peer.enqueue(
            json!({"v":1,"id":"f","type":"feedback","feedback":feedback(&service, &"a".repeat(64))})
        ));
        // A partially sent frame must not resume after disconnect either.
        peer.socket.write_all(&peer.writes[0].bytes[..2]).unwrap();
        peer.writes[0].offset = 2;
        client.read_exact(&mut [0; 2]).unwrap();
        service.disconnect();
        assert_eq!(
            peer.flush_current(&service.shared).unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
        assert!(peer.writes.is_empty());
        drop(peer);
        assert_closed(&mut client);
        // Retiring the old generation must not prevent a new authenticated peer.
        let mut socket = connect(&service);
        send(&mut socket, json!({"v":1,"id":"s","type":"status"}));
        assert_eq!(receive(&mut socket)["status"]["session_id"], "test");
    }
    #[test]
    fn fragmented_frames_and_partial_write_resume() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        let mut peer = Peer::new(accepted).unwrap();
        client.set_nodelay(true).unwrap();
        let body = b"{\"v\":1}";
        let header = (body.len() as u32).to_be_bytes();
        let partial = |peer: &mut Peer, header_read, body_read| {
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                assert!(peer.read_frame().unwrap().is_none());
                if peer.header_read == header_read && peer.body_read == body_read {
                    break;
                }
                assert!(Instant::now() < deadline, "partial frame did not arrive");
                thread::sleep(Duration::from_millis(1));
            }
        };
        client.write_all(&header[..1]).unwrap();
        partial(&mut peer, 1, 0);
        client.write_all(&header[1..]).unwrap();
        partial(&mut peer, 4, 0);
        client.write_all(&body[..2]).unwrap();
        partial(&mut peer, 4, 2);
        client.write_all(&body[2..]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(frame) = peer.read_frame().unwrap() {
                assert_eq!(frame, body);
                break;
            }
            assert!(Instant::now() < deadline, "complete frame did not arrive");
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(peer.header_read, 0);
        assert!(peer.enqueue(json!({"v":1})));
        let prefix = peer.writes.front().unwrap().bytes[..2].to_vec();
        peer.socket.write_all(&prefix).unwrap();
        peer.writes.front_mut().unwrap().offset = 2;
        let deadline = Instant::now() + Duration::from_secs(1);
        while !peer.writes.is_empty() {
            peer.flush().unwrap();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(receive(&mut client), json!({"v":1}));
    }
    #[test]
    fn write_queue_and_incomplete_authenticated_frame() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        let mut peer = Peer::new(accepted).unwrap();
        for _ in 0..32 {
            assert!(peer.enqueue(json!({"v":1})));
        }
        assert!(!peer.enqueue(json!({"v":1})));
        peer.writes.front_mut().unwrap().started = Instant::now() - Duration::from_secs(3);
        assert_eq!(peer.flush().unwrap_err().kind(), io::ErrorKind::TimedOut);
        socket.write_all(&[0]).unwrap();
        assert!(peer.read_frame().unwrap().is_none());
        peer.frame_started = Some(Instant::now() - Duration::from_secs(3));
        assert_eq!(
            peer.read_frame().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }
    #[test]
    fn authenticated_status_and_logs_contract() {
        let service = DeveloperService::start().unwrap();
        let mut socket = connect(&service);
        send(&mut socket, json!({"v":1,"id":"s","type":"status"}));
        let status = receive(&mut socket);
        assert_eq!(status["status"]["session_id"], "test");
        send(&mut socket, json!({"v":1,"id":"l","type":"logs","limit":1}));
        assert_eq!(receive(&mut socket)["type"], "logs_result");
        send(
            &mut socket,
            json!({"v":1,"id":"l","type":"logs","after":{"capture_id":"old","sequence":0}}),
        );
        assert_eq!(receive(&mut socket)["code"], "capture_changed");
    }
    #[test]
    fn bad_token_and_oversized_prefix_close() {
        let service = DeveloperService::start().unwrap();
        for oversized in [false, true] {
            let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, service.port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            if oversized {
                s.write_all(&65537u32.to_be_bytes()).unwrap();
            } else {
                send(
                    &mut s,
                    json!({"v":1,"id":"h","type":"hello","token":"0".repeat(64),"session_id":"x","workspace":"x"}),
                );
            }
            assert_closed(&mut s);
        }
        let _ = connect(&service);
    }
    #[test]
    fn idle_pending_does_not_reserve_slot_and_shutdown_bounded() {
        let service = DeveloperService::start().unwrap();
        let pending = TcpStream::connect((Ipv4Addr::LOCALHOST, service.port)).unwrap();
        let _paired = connect(&service);
        let now = Instant::now();
        drop(service);
        assert!(now.elapsed() < Duration::from_secs(1));
        drop(pending);
    }
    #[test]
    fn pending_handshakes_and_partial_deadline() {
        let service = DeveloperService::start().unwrap();
        let mut peers = Vec::new();
        for _ in 0..4 {
            peers.push(TcpStream::connect((Ipv4Addr::LOCALHOST, service.port)).unwrap());
        }
        thread::sleep(Duration::from_millis(50));
        let mut excess = TcpStream::connect((Ipv4Addr::LOCALHOST, service.port)).unwrap();
        excess
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert_closed(&mut excess);
        peers[0].write_all(&[0]).unwrap();
        thread::sleep(Duration::from_millis(2100));
        for mut p in peers {
            p.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            assert_closed(&mut p);
        }
        let _ = connect(&service);
    }
}
