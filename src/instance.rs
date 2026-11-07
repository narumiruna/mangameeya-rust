//! Per-user, authenticated launch forwarding. An empty launch request means focus.
//!
//! Keep `PrimaryInstance` alive for the lifetime of the application and poll
//! `try_recv` on the UI thread. The queue includes the primary's initial launch.

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_PATHS: usize = 128;
const MAX_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_ENDPOINT_BYTES: u64 = 1024;
const QUEUE_CAPACITY: usize = 32;
const IO_TIMEOUT: Duration = Duration::from_millis(500);
const START_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const ENDPOINT_NAME: &str = "instance.json";

pub enum Instance {
    Primary(PrimaryInstance),
    Forwarded,
}

pub struct PrimaryInstance {
    receiver: Receiver<Vec<PathBuf>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    endpoint_path: PathBuf,
    token: String,
    // Never delete the lock file: an open, unlinked inode can admit two primaries.
    _lock_file: File,
}

impl PrimaryInstance {
    /// Each request must activate the window, including requests with no paths.
    pub fn try_recv(&self) -> Option<Vec<PathBuf>> {
        self.receiver.try_recv().ok()
    }
}

impl Drop for PrimaryInstance {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if read_endpoint(&self.endpoint_path).is_ok_and(|endpoint| endpoint.token == self.token) {
            let _ = fs::remove_file(&self.endpoint_path);
        }
        // The file lock is released only after the listener and endpoint are gone.
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    port: u16,
    pid: u32,
    token: String,
}

// Preserve native file names, including non-UTF-8 Unix names and Windows UTF-16.
#[cfg(unix)]
type WirePath = Vec<u8>;
#[cfg(windows)]
type WirePath = Vec<u16>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    paths: Vec<WirePath>,
}

/// Start a primary instance, or send this launch to the existing primary.
///
/// `config_dir` must be the application's per-user configuration directory.
/// Relative paths are resolved against this process's working directory, not
/// the primary's. Paths need not exist. No command other than launch is exposed.
pub fn start(config_dir: &Path, initial_paths: Vec<PathBuf>) -> Result<Instance, String> {
    if initial_paths.len() > MAX_PATHS {
        return Err(format!("A launch can contain at most {MAX_PATHS} paths."));
    }
    let paths = initial_paths
        .into_iter()
        .map(|path| std::path::absolute(path).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    // Apply the same payload limit to initial and forwarded launches.
    encode_request(&"0".repeat(64), &paths)?;
    create_private_dir(config_dir).map_err(|error| format!("Create config directory: {error}"))?;
    let lock_file = private_options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(config_dir.join("instance.lock"))
        .map_err(|error| format!("Open instance lock: {error}"))?;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match FileExt::try_lock_exclusive(&lock_file) {
            Ok(()) => return start_primary(config_dir, lock_file, paths).map(Instance::Primary),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
            Err(error) => return Err(format!("Lock application instance: {error}")),
        }
        if let Ok(endpoint) = read_endpoint(&config_dir.join(ENDPOINT_NAME)) {
            allow_primary_focus(endpoint.pid);
            let message = encode_request(&endpoint.token, &paths)?;
            if send_request(endpoint.port, &message).is_ok() {
                return Ok(Instance::Forwarded);
            }
        }
        if Instant::now() >= deadline {
            return Err(
                "The running instance did not accept this launch. Close it and try again."
                    .to_string(),
            );
        }
        // Retry the lock as well: the old primary can exit during forwarding.
        thread::sleep(POLL_INTERVAL);
    }
}

fn start_primary(
    config_dir: &Path,
    lock_file: File,
    initial_paths: Vec<PathBuf>,
) -> Result<PrimaryInstance, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| format!("Create launch listener: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|error| format!("Create launch token: {error}"))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let token: String = random
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 15) as usize] as char,
            ]
        })
        .collect();
    let endpoint = Endpoint {
        port: listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port(),
        pid: std::process::id(),
        token: token.clone(),
    };
    let endpoint_path = config_dir.join(ENDPOINT_NAME);
    write_endpoint(config_dir, &endpoint)
        .map_err(|error| format!("Publish launch endpoint: {error}"))?;
    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    // A new, empty queue cannot reject this initial request.
    let _ = sender.try_send(initial_paths);
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker_token = token.clone();
    let worker = thread::Builder::new()
        .name("launch-ipc".to_string())
        .spawn(move || serve(listener, sender, worker_token, worker_stop))
        .map_err(|error| {
            let _ = fs::remove_file(&endpoint_path);
            format!("Start launch listener: {error}")
        })?;
    Ok(PrimaryInstance {
        receiver,
        stop,
        worker: Some(worker),
        endpoint_path,
        token,
        _lock_file: lock_file,
    })
}

fn private_options() -> OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = OpenOptions::new();
        options.mode(0o600);
        options
    }
    #[cfg(not(unix))]
    {
        OpenOptions::new()
    }
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    // Windows inherits the ACL from the user's profile directory.
    Ok(())
}

fn write_endpoint(dir: &Path, endpoint: &Endpoint) -> io::Result<()> {
    let pending = dir.join("instance.pending");
    match fs::remove_file(&pending) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let result = (|| {
        let mut file = private_options()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        serde_json::to_writer(&mut file, endpoint)?;
        file.flush()?;
        drop(file);
        fs::rename(&pending, dir.join(ENDPOINT_NAME))
    })();
    if result.is_err() {
        let _ = fs::remove_file(pending);
    }
    result
}

fn read_endpoint(path: &Path) -> io::Result<Endpoint> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_ENDPOINT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ENDPOINT_BYTES {
        return Err(io::Error::other("Launch endpoint exceeds its size limit"));
    }
    let endpoint: Endpoint = serde_json::from_slice(&bytes)?;
    if endpoint.port == 0
        || endpoint.pid == 0
        || endpoint.token.len() != 64
        || !endpoint.token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(io::Error::other("Invalid launch endpoint"));
    }
    Ok(endpoint)
}

fn encode_request(token: &str, paths: &[PathBuf]) -> Result<Vec<u8>, String> {
    let message = serde_json::to_vec(&Request {
        token: token.to_string(),
        paths: paths.iter().map(|path| path_to_wire(path)).collect(),
    })
    .map_err(|error| format!("Encode launch paths: {error}"))?;
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(format!(
            "Launch paths exceed the {MAX_MESSAGE_BYTES}-byte message limit."
        ));
    }
    Ok(message)
}

#[cfg(unix)]
fn path_to_wire(path: &Path) -> WirePath {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn path_to_wire(path: &Path) -> WirePath {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().collect()
}

#[cfg(unix)]
fn path_from_wire(path: WirePath) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(path).into()
}

#[cfg(windows)]
fn path_from_wire(path: WirePath) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    std::ffi::OsString::from_wide(&path).into()
}

#[cfg(windows)]
fn allow_primary_focus(pid: u32) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn AllowSetForegroundWindow(process_id: u32) -> i32;
    }
    // The launch process can grant the existing process permission to activate.
    // Windows can deny this; failure must not prevent path forwarding.
    // SAFETY: This system API takes a DWORD process ID and no pointers.
    unsafe {
        let _ = AllowSetForegroundWindow(pid);
    }
}

#[cfg(not(windows))]
fn allow_primary_focus(_: u32) {}

fn send_request(port: u16, message: &[u8]) -> io::Result<()> {
    let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.write_all(&(message.len() as u32).to_be_bytes())?;
    stream.write_all(message)?;
    let mut accepted = [0_u8];
    stream.read_exact(&mut accepted)?;
    if accepted != [1] {
        return Err(io::Error::other("Launch request was not accepted"));
    }
    Ok(())
}

fn serve(
    listener: TcpListener,
    sender: SyncSender<Vec<PathBuf>>,
    token: String,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, address)) if address.ip().is_loopback() => {
                // BSD sockets can inherit the listener's nonblocking mode.
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
                let accepted = receive_request(&mut stream, &token)
                    .is_ok_and(|paths| sender.try_send(paths).is_ok());
                let _ = stream.write_all(&[u8::from(accepted)]);
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL_INTERVAL),
            Err(_) => break,
        }
    }
}

fn receive_request(stream: &mut TcpStream, token: &str) -> io::Result<Vec<PathBuf>> {
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut header = [0_u8; 4];
    read_before(stream, &mut header, deadline)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_MESSAGE_BYTES {
        return Err(io::Error::other("Launch request exceeds its size limit"));
    }
    let mut bytes = vec![0; length];
    read_before(stream, &mut bytes, deadline)?;
    let request: Request = serde_json::from_slice(&bytes)?;
    if !token_matches(&request.token, token) || request.paths.len() > MAX_PATHS {
        return Err(io::Error::other("Invalid launch request"));
    }
    let paths: Vec<PathBuf> = request.paths.into_iter().map(path_from_wire).collect();
    if paths.iter().any(|path| !path.is_absolute()) {
        return Err(io::Error::other("Launch paths must be absolute"));
    }
    Ok(paths)
}

fn token_matches(candidate: &str, expected: &str) -> bool {
    if candidate.len() != expected.len() {
        return false;
    }
    candidate
        .bytes()
        .zip(expected.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn read_before(stream: &mut TcpStream, mut buffer: &mut [u8], deadline: Instant) -> io::Result<()> {
    // A total deadline prevents a peer from keeping the listener busy by sending
    // one byte before each socket timeout. There are no per-client threads.
    while !buffer.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Launch request timed out",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        match stream.read(buffer) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Incomplete launch",
                ));
            }
            Ok(count) => buffer = &mut buffer[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random).unwrap();
            let name = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            Self(std::env::temp_dir().join(format!("mangameeya-instance-{name}")))
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn primary(dir: &Path, paths: Vec<PathBuf>) -> PrimaryInstance {
        match start(dir, paths).unwrap() {
            Instance::Primary(primary) => primary,
            Instance::Forwarded => panic!("Expected primary"),
        }
    }

    fn receive(primary: &PrimaryInstance) -> Vec<PathBuf> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(paths) = primary.try_recv() {
                return paths;
            }
            assert!(Instant::now() < deadline, "Launch request was not received");
            thread::sleep(POLL_INTERVAL);
        }
    }

    #[test]
    fn forwards_paths_and_empty_focus_requests() {
        let dir = TestDir::new();
        let relative = PathBuf::from("chapter 1.cbz");
        let absolute = std::path::absolute(&relative).unwrap();
        let primary = primary(&dir.0, vec![relative.clone()]);
        assert_eq!(receive(&primary), vec![absolute.clone()]);
        assert!(matches!(
            start(&dir.0, vec![relative]).unwrap(),
            Instance::Forwarded
        ));
        assert_eq!(receive(&primary), vec![absolute]);
        assert!(matches!(
            start(&dir.0, vec![]).unwrap(),
            Instance::Forwarded
        ));
        assert!(receive(&primary).is_empty());
    }

    #[test]
    fn rejects_wrong_authentication_and_relative_wire_paths() {
        let dir = TestDir::new();
        let primary = primary(&dir.0, vec![]);
        receive(&primary);
        let endpoint = read_endpoint(&primary.endpoint_path).unwrap();
        let wrong_token = if endpoint.token.starts_with('0') {
            "1"
        } else {
            "0"
        }
        .repeat(64);
        assert!(send_request(endpoint.port, &encode_request(&wrong_token, &[]).unwrap()).is_err());
        let relative = encode_request(&endpoint.token, &[PathBuf::from("chapter.cbz")]).unwrap();
        assert!(send_request(endpoint.port, &relative).is_err());
        let unknown_command = serde_json::to_vec(&serde_json::json!({
            "token": endpoint.token,
            "paths": [],
            "command": "unsupported"
        }))
        .unwrap();
        assert!(send_request(endpoint.port, &unknown_command).is_err());
        assert!(primary.try_recv().is_none());
        assert!(matches!(
            start(&dir.0, vec![]).unwrap(),
            Instance::Forwarded
        ));
        assert!(receive(&primary).is_empty());
    }

    #[test]
    fn rejects_oversized_payloads_and_path_counts() {
        let dir = TestDir::new();
        let primary = primary(&dir.0, vec![]);
        receive(&primary);
        assert!(start(&dir.0, vec![dir.0.clone(); MAX_PATHS + 1]).is_err());
        assert!(
            encode_request(
                &primary.token,
                &[PathBuf::from("x".repeat(MAX_MESSAGE_BYTES))]
            )
            .is_err()
        );
        let endpoint = read_endpoint(&primary.endpoint_path).unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .write_all(&((MAX_MESSAGE_BYTES + 1) as u32).to_be_bytes())
            .unwrap();
        let mut response = [1];
        stream.read_exact(&mut response).unwrap();
        assert_eq!(response, [0]);
        let too_many =
            encode_request(&endpoint.token, &vec![dir.0.clone(); MAX_PATHS + 1]).unwrap();
        assert!(send_request(endpoint.port, &too_many).is_err());
        assert!(primary.try_recv().is_none());
    }

    #[test]
    fn queue_is_bounded_and_rejects_overflow() {
        let dir = TestDir::new();
        let primary = primary(&dir.0, vec![]);
        receive(&primary);
        let endpoint = read_endpoint(&primary.endpoint_path).unwrap();
        let request = encode_request(&endpoint.token, &[]).unwrap();
        for _ in 0..QUEUE_CAPACITY {
            send_request(endpoint.port, &request).unwrap();
        }
        assert!(send_request(endpoint.port, &request).is_err());
        for _ in 0..QUEUE_CAPACITY {
            assert!(receive(&primary).is_empty());
        }
        assert!(primary.try_recv().is_none());
    }

    #[test]
    fn cleanup_keeps_lock_inode_and_replaces_stale_endpoint() {
        let dir = TestDir::new();
        let first = primary(&dir.0, vec![]);
        let token = first.token.clone();
        let endpoint_path = first.endpoint_path.clone();
        assert!(endpoint_path.exists());
        assert_eq!(
            read_endpoint(&endpoint_path).unwrap().pid,
            std::process::id()
        );
        drop(first);
        assert!(!endpoint_path.exists());
        assert!(dir.0.join("instance.lock").exists());
        fs::write(&endpoint_path, b"stale data").unwrap();
        fs::write(dir.0.join("instance.pending"), b"incomplete write").unwrap();
        let next = primary(&dir.0, vec![]);
        assert_ne!(next.token, token);
        assert!(!dir.0.join("instance.pending").exists());
        assert_eq!(read_endpoint(&endpoint_path).unwrap().token, next.token);
    }

    #[test]
    fn simultaneous_launches_have_one_primary() {
        let dir = TestDir::new();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let path = dir.0.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    start(&path, vec![]).unwrap()
                })
            })
            .collect();
        let instances: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(
            instances
                .iter()
                .filter(|instance| matches!(instance, Instance::Primary(_)))
                .count(),
            1
        );
        assert_eq!(
            instances
                .iter()
                .filter(|instance| matches!(instance, Instance::Forwarded))
                .count(),
            1
        );
        for instance in &instances {
            if let Instance::Primary(primary) = instance {
                assert!(receive(primary).is_empty());
                assert!(receive(primary).is_empty());
            }
        }
    }

    #[test]
    fn forwards_from_a_separate_process_with_its_own_working_directory() {
        let dir = TestDir::new();
        let primary = primary(&dir.0, vec![]);
        receive(&primary);
        let test_module = module_path!().split_once("::").unwrap().1;
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(format!("{test_module}::subprocess_forwarding_entry"))
            .env("MANGAMEEYA_INSTANCE_TEST_DIR", &dir.0)
            .current_dir(&dir.0)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let forwarded = receive(&primary);
        assert_eq!(forwarded.len(), 1);
        assert!(forwarded[0].is_absolute());
        assert_eq!(forwarded[0].file_name().unwrap(), "child-launch.cbz");
        // macOS getcwd resolves /var to /private/var; Windows can add a verbatim prefix.
        assert_eq!(
            fs::canonicalize(forwarded[0].parent().unwrap()).unwrap(),
            fs::canonicalize(&dir.0).unwrap()
        );
    }

    #[test]
    fn subprocess_forwarding_entry() {
        let Some(dir) = std::env::var_os("MANGAMEEYA_INSTANCE_TEST_DIR") else {
            return;
        };
        assert!(matches!(
            start(Path::new(&dir), vec![PathBuf::from("child-launch.cbz")]).unwrap(),
            Instance::Forwarded
        ));
    }

    #[cfg(unix)]
    #[test]
    fn native_paths_round_trip_and_endpoint_is_private() {
        use std::os::unix::ffi::OsStringExt;
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new();
        let primary = primary(&dir.0, vec![]);
        receive(&primary);
        let path = dir
            .0
            .join(std::ffi::OsString::from_vec(b"chapter-\xff.cbz".to_vec()));
        assert!(matches!(
            start(&dir.0, vec![path.clone()]).unwrap(),
            Instance::Forwarded
        ));
        assert_eq!(receive(&primary), vec![path]);
        assert_eq!(
            fs::metadata(&primary.endpoint_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&dir.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_utf16_paths_round_trip() {
        use std::os::windows::ffi::OsStringExt;
        let dir = TestDir::new();
        let primary = primary(&dir.0, vec![]);
        receive(&primary);
        let name = std::ffi::OsString::from_wide(&[
            b'p' as u16,
            b'a' as u16,
            b'g' as u16,
            b'e' as u16,
            0xd800,
            b'.' as u16,
            b'p' as u16,
            b'n' as u16,
            b'g' as u16,
        ]);
        let path = dir.0.join(name);
        assert!(matches!(
            start(&dir.0, vec![path.clone()]).unwrap(),
            Instance::Forwarded
        ));
        assert_eq!(receive(&primary), vec![path]);
    }

    #[test]
    fn incomplete_request_does_not_prevent_shutdown() {
        let dir = TestDir::new();
        let primary = primary(&dir.0, vec![]);
        let endpoint = read_endpoint(&primary.endpoint_path).unwrap();
        let _stream = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port)).unwrap();
        thread::sleep(POLL_INTERVAL * 2);
        let before = Instant::now();
        drop(primary);
        assert!(before.elapsed() < Duration::from_secs(2));
        assert!(!dir.0.join(ENDPOINT_NAME).exists());
    }
}
