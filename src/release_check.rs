//! Blocking, notification-only release checks and separate notification preferences.

use semver::Version;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;
use tempfile::NamedTempFile;

pub const RELEASES_URL: &str = "https://github.com/narumiruna/mangameeya-rust/releases";
const API_URL: &str = "https://api.github.com/repos/narumiruna/mangameeya-rust/releases/latest";
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RELEASE_BYTES: u64 = 1024 * 1024;
const MAX_TAG_BYTES: usize = 128;
const MAX_PREFERENCES_BYTES: u64 = 16 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Release {
    /// Semantic version without the optional leading `v`.
    pub version: String,
}

/// Run on a background executor. This does not download or install releases.
/// Only the fixed GitHub API endpoint is used; response URLs are ignored.
pub fn check_latest() -> Result<Option<Release>, String> {
    read_response(release_request(API_URL).call(), env!("CARGO_PKG_VERSION"))
}

fn release_request(url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(REQUEST_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .build()
        .into();
    agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
}

fn read_response(
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    current_version: &str,
) -> Result<Option<Release>, String> {
    let mut response = response.map_err(|error| format!("Release check failed: {error}"))?;
    match response.status().as_u16() {
        // GitHub returns 404 when no published release exists.
        404 => Ok(None),
        200 => {
            let bytes = read_bounded(
                response.body_mut().as_reader(),
                MAX_RELEASE_BYTES,
                "Release response",
            )?;
            newer_release(&bytes, current_version)
        }
        status => Err(format!("Release check failed: HTTP {status}")),
    }
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn newer_release(bytes: &[u8], current_version: &str) -> Result<Option<Release>, String> {
    if !bytes.trim_ascii_start().starts_with(b"{") {
        return Err("Invalid release response: expected a JSON object".to_owned());
    }
    let release: ApiRelease = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid release response: {error}"))?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    if release.tag_name.len() > MAX_TAG_BYTES {
        return Err(format!(
            "Release tag exceeds the {MAX_TAG_BYTES} byte limit"
        ));
    }
    let tag = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let latest =
        Version::parse(tag).map_err(|error| format!("Invalid release version: {error}"))?;
    if !latest.pre.is_empty() {
        return Ok(None);
    }
    let current = Version::parse(current_version)
        .map_err(|error| format!("Invalid application version: {error}"))?;
    // Version's total ordering includes build metadata; SemVer precedence does not.
    Ok(latest.cmp_precedence(&current).is_gt().then(|| Release {
        version: latest.to_string(),
    }))
}

fn read_bounded(reader: impl Read, limit: u64, description: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read {description}: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{description} exceeds the {limit} byte limit"));
    }
    Ok(bytes)
}

/// Store only in the separate `release-notifications.json` file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationPreferences {
    pub check_on_startup: bool,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            check_on_startup: true,
        }
    }
}

impl NotificationPreferences {
    /// A missing file uses defaults and does not create a file or directory.
    pub fn load_from(path: &Path) -> Result<Self, String> {
        let description = format!("notification preferences {}", path.display());
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("Cannot read {description}: {error}")),
        };
        let bytes = read_bounded(file, MAX_PREFERENCES_BYTES, &description)?;
        let invalid = |error| format!("Invalid {description}: {error}");
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
        if !value.is_object() {
            return Err(invalid("expected a JSON object".to_owned()));
        }
        serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))
    }

    /// Atomically replace this notification-only file after an explicit save.
    /// An existing corrupt file can be replaced; no reader or book data is read.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("Cannot encode notification preferences: {error}"))?;
        bytes.push(b'\n');
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Cannot create notification preferences directory {}: {error}",
                parent.display()
            )
        })?;
        let mut temporary = NamedTempFile::new_in(parent).map_err(|error| {
            format!("Cannot create temporary notification preferences: {error}")
        })?;
        temporary
            .write_all(&bytes)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|error| format!("Cannot write notification preferences: {error}"))?;
        temporary.persist(path).map_err(|error| {
            format!(
                "Cannot replace notification preferences {}: {error}",
                path.display()
            )
        })?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("Cannot sync notification preferences directory: {error}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc::{self, Sender};
    use std::thread::{self, JoinHandle};
    use std::time::Instant;

    fn release_json(tag: &str, draft: bool, prerelease: bool) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "tag_name": tag,
            "draft": draft,
            "prerelease": prerelease,
        }))
        .unwrap()
    }

    #[test]
    fn versions_use_semver_precedence_and_ignore_build_metadata() {
        for (current, tag, expected) in [
            ("0.7.0", "v0.8.0", Some("0.8.0")),
            ("0.7.0", "0.8.0", Some("0.8.0")),
            ("0.9.0", "v0.10.0", Some("0.10.0")),
            ("0.10.0", "v0.9.0", None),
            ("0.7.0", "v1.0.0", Some("1.0.0")),
            ("1.0.0", "v0.99.99", None),
            ("0.7.0", "v0.7.0", None),
            ("0.7.0", "v0.7.0+new", None),
            ("0.7.0+old", "v0.7.0+new", None),
            ("0.7.0+new", "v0.7.0", None),
            ("0.7.0-rc.1", "v0.7.0", Some("0.7.0")),
            ("0.7.0", "v0.8.0+build.1", Some("0.8.0+build.1")),
        ] {
            let actual = newer_release(&release_json(tag, false, false), current).unwrap();
            assert_eq!(
                actual,
                expected.map(|version| Release {
                    version: version.to_owned()
                }),
                "current={current}, tag={tag}"
            );
        }
    }

    #[test]
    fn drafts_and_all_prereleases_are_ignored() {
        for (tag, draft, prerelease) in [
            ("v9.0.0", true, false),
            ("v9.0.0", false, true),
            ("v9.0.0", true, true),
            ("v9.0.0-alpha.1", false, false),
            ("9.0.0-rc.1+build", false, false),
            ("not-a-version", true, false),
            ("not-a-version", false, true),
        ] {
            assert_eq!(
                newer_release(&release_json(tag, draft, prerelease), "0.7.0"),
                Ok(None)
            );
        }
    }

    #[test]
    fn only_an_optional_lowercase_v_prefix_is_accepted() {
        for tag in [
            "",
            "V1.0.0",
            "vv1.0.0",
            "release/1.0.0",
            "1.0",
            "01.0.0",
            " 1.0.0",
            "1.0.0 ",
            "1.0.0\n",
        ] {
            assert!(
                newer_release(&release_json(tag, false, false), "0.7.0")
                    .unwrap_err()
                    .contains("Invalid release version"),
                "{tag:?}"
            );
        }
        assert!(
            newer_release(&release_json("v1.0.0", false, false), "invalid")
                .unwrap_err()
                .contains("Invalid application version")
        );
    }

    #[test]
    fn release_tags_are_bounded_before_version_parsing() {
        let mut tag = "v1.0.0+".to_owned();
        tag.push_str(&"a".repeat(MAX_TAG_BYTES - tag.len()));
        assert!(
            newer_release(&release_json(&tag, false, false), "0.7.0")
                .unwrap()
                .is_some()
        );
        tag.push('a');
        assert!(
            newer_release(&release_json(&tag, false, false), "0.7.0")
                .unwrap_err()
                .contains("128 byte limit")
        );
        let tag = format!("v1.0.0-{}", "a".repeat(MAX_TAG_BYTES));
        assert!(
            newer_release(&release_json(&tag, false, false), "0.7.0")
                .unwrap_err()
                .contains("128 byte limit")
        );
    }

    #[test]
    fn malformed_release_data_returns_errors() {
        for bytes in [
            b"{".as_slice(),
            b"null",
            b"[]",
            br#"["v1.0.0",false,false]"#,
            b"{}",
            br#"{"tag_name":"v1.0.0"}"#,
            br#"{"tag_name":1,"draft":false,"prerelease":false}"#,
            br#"{"tag_name":"v1.0.0","draft":null,"prerelease":false}"#,
            br#"{"tag_name":"v1.0.0","draft":false,"prerelease":"false"}"#,
            br#"{"tag_name":"v1.0.0","draft":false,"prerelease":false,"tag_name":"v2.0.0"}"#,
            &[0xff, 0xfe],
        ] {
            assert!(
                newer_release(bytes, "0.7.0")
                    .unwrap_err()
                    .contains("Invalid release response")
            );
        }
    }

    #[test]
    fn response_urls_are_ignored() {
        let bytes = br#"{"tag_name":"v0.8.0","draft":false,"prerelease":false,"html_url":"https://untrusted.invalid/","assets":[{"browser_download_url":"file:///tmp/untrusted"}]}"#;
        assert_eq!(
            newer_release(bytes, "0.7.0"),
            Ok(Some(Release {
                version: "0.8.0".to_owned()
            }))
        );
        assert_eq!(
            RELEASES_URL,
            "https://github.com/narumiruna/mangameeya-rust/releases"
        );
    }

    #[test]
    fn bounded_reads_accept_the_limit_and_reject_extra_bytes_and_io_errors() {
        assert_eq!(read_bounded(&b"1234"[..], 4, "test").unwrap(), b"1234");
        assert!(
            read_bounded(&b"12345"[..], 4, "test")
                .unwrap_err()
                .contains("exceeds")
        );
        struct FailedReader;
        impl Read for FailedReader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("test read failure"))
            }
        }
        assert!(
            read_bounded(FailedReader, 4, "test")
                .unwrap_err()
                .contains("test read failure")
        );
    }

    // Accept only a loopback connection, with bounded waits and no environment proxy.
    fn serve_once(response: Vec<u8>) -> (String, JoinHandle<String>, Sender<()>) {
        let (done, wait_until_done) = mpsc::channel();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/latest", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "No local request received");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("Local accept failed: {error}"),
                }
            };
            // Accepted sockets can inherit the listener's nonblocking mode.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() <= 16 * 1024);
            }
            // Oversized responses can make the client close before this write ends.
            let _ = stream.write_all(&response);
            // The caller closes the channel after it reads the response. The
            // timeout also lets a truncated response finish without a deadlock.
            let _ = wait_until_done.recv_timeout(Duration::from_secs(2));
            String::from_utf8(request).unwrap()
        });
        (url, server, done)
    }

    fn http_response(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn local_check(response: Vec<u8>) -> (Result<Option<Release>, String>, String) {
        let (url, server, done) = serve_once(response);
        let response = release_request(&url)
            .config()
            .https_only(false)
            .proxy(None)
            .build()
            .call();
        let result = read_response(response, "0.7.0");
        drop(done);
        (result, server.join().unwrap())
    }

    #[test]
    fn http_check_sends_github_headers_and_reads_the_release() {
        let (result, request) = local_check(http_response(
            "200 OK",
            "Content-Type: application/json\r\n",
            &release_json("v0.8.0", false, false),
        ));
        assert_eq!(
            result,
            Ok(Some(Release {
                version: "0.8.0".to_owned()
            }))
        );
        let request = request.to_ascii_lowercase();
        assert!(request.starts_with("get /latest http/1.1\r\n"));
        assert!(request.contains(&format!("user-agent: {USER_AGENT}\r\n")));
        assert!(request.contains("accept: application/vnd.github+json\r\n"));
        assert!(request.contains("x-github-api-version: 2022-11-28\r\n"));
    }

    #[test]
    fn http_not_found_means_no_published_release() {
        assert_eq!(
            local_check(http_response("404 Not Found", "", b"not JSON")).0,
            Ok(None)
        );
    }

    #[test]
    fn http_errors_and_redirects_are_not_accepted_or_followed() {
        for status in [
            "301 Moved Permanently",
            "302 Found",
            "403 Forbidden",
            "429 Too Many Requests",
            "500 Internal Server Error",
            "204 No Content",
        ] {
            let (result, _) = local_check(http_response(
                status,
                "Location: https://must-not-be-requested.invalid/\r\n",
                b"",
            ));
            assert!(
                result
                    .unwrap_err()
                    .contains(&format!("HTTP {}", &status[..3])),
                "{status}"
            );
        }
    }

    #[test]
    fn http_oversized_and_truncated_bodies_return_errors() {
        let oversized = vec![b' '; MAX_RELEASE_BYTES as usize + 1];
        let error = local_check(http_response("200 OK", "", &oversized))
            .0
            .unwrap_err();
        assert!(error.contains("exceeds"), "{error}");
        let (result, _) = local_check(
            b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 100\r\n\r\n{".to_vec(),
        );
        assert!(result.unwrap_err().contains("Cannot read Release response"));
    }

    #[test]
    fn global_deadline_applies_to_body_reads() {
        let (url, server, done) = serve_once(
            b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 100\r\n\r\n{".to_vec(),
        );
        let response = release_request(&url)
            .config()
            .https_only(false)
            .proxy(None)
            .timeout_global(Some(Duration::from_millis(500)))
            .build()
            .call()
            .unwrap();
        let error = read_response(Ok(response), "0.7.0").unwrap_err();
        assert!(error.contains("Cannot read Release response"), "{error}");
        assert!(error.contains("timeout"), "{error}");
        drop(done);
        server.join().unwrap();
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(10));
    }

    #[test]
    fn production_request_rejects_plain_http() {
        // Rejected before any connection; this does not require a local server.
        let result = read_response(release_request("http://127.0.0.1:1/latest").call(), "0.7.0");
        assert!(result.is_err());
    }

    #[test]
    fn transport_errors_are_returned() {
        let response = Err(ureq::Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "offline test",
        )));
        let error = read_response(response, "0.7.0").unwrap_err();
        assert!(error.contains("Release check failed"));
        assert!(error.contains("offline test"));
    }

    #[test]
    fn preferences_default_to_startup_checks_and_serialize() {
        let preferences = NotificationPreferences::default();
        assert!(preferences.check_on_startup);
        assert_eq!(
            serde_json::from_str::<NotificationPreferences>("{}").unwrap(),
            preferences
        );
        assert_eq!(
            serde_json::to_value(&preferences).unwrap(),
            serde_json::json!({"check_on_startup": true})
        );
    }

    #[test]
    fn missing_preferences_do_not_create_files_or_directories() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing/release-notifications.json");
        assert_eq!(
            NotificationPreferences::load_from(&path).unwrap(),
            NotificationPreferences::default()
        );
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn preferences_round_trip_and_replace_without_changing_reader_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config/release-notifications.json");
        let preferences = NotificationPreferences {
            check_on_startup: false,
        };
        preferences.save_to(&path).unwrap();
        assert_eq!(
            NotificationPreferences::load_from(&path).unwrap(),
            preferences
        );
        let settings_path = path.with_file_name("settings.json");
        let reader_data = br#"{"recent":["book.cbz"],"positions":{"book.cbz":5}}"#;
        fs::write(&settings_path, reader_data).unwrap();
        NotificationPreferences::default().save_to(&path).unwrap();
        assert_eq!(
            NotificationPreferences::load_from(&path).unwrap(),
            NotificationPreferences::default()
        );
        assert_eq!(fs::read(settings_path).unwrap(), reader_data);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 2);
    }

    #[test]
    fn malformed_preferences_remain_unchanged_until_an_explicit_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("release-notifications.json");
        for bytes in [
            b"{".as_slice(),
            b"null",
            b"[]",
            b"[false]",
            br#"{"check_on_startup":null}"#,
            br#"{"check_on_startup":"false"}"#,
            br#"{"check_on_startup":0}"#,
            br#"{"check_on_startup":true,"check_on_startup":false}"#,
            br#"{"unknown":false}"#,
            &[0xff, 0xfe],
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(NotificationPreferences::load_from(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            NotificationPreferences::default().save_to(&path).unwrap();
            assert_eq!(
                NotificationPreferences::load_from(&path).unwrap(),
                NotificationPreferences::default()
            );
        }
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn preferences_reads_are_bounded_and_explicit_saves_can_replace_oversized_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("release-notifications.json");
        let mut bytes = b"{}".to_vec();
        bytes.resize(MAX_PREFERENCES_BYTES as usize, b' ');
        fs::write(&path, &bytes).unwrap();
        assert_eq!(
            NotificationPreferences::load_from(&path).unwrap(),
            NotificationPreferences::default()
        );
        bytes.push(b' ');
        fs::write(&path, &bytes).unwrap();
        assert!(
            NotificationPreferences::load_from(&path)
                .unwrap_err()
                .contains("exceeds")
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let preferences = NotificationPreferences {
            check_on_startup: false,
        };
        preferences.save_to(&path).unwrap();
        assert_eq!(
            NotificationPreferences::load_from(&path).unwrap(),
            preferences
        );
    }

    #[test]
    fn preference_storage_errors_preserve_existing_data_and_remove_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-a-directory");
        fs::write(&path, b"keep").unwrap();
        assert!(NotificationPreferences::load_from(directory.path()).is_err());
        assert!(
            NotificationPreferences::default()
                .save_to(&path.join("release-notifications.json"))
                .is_err()
        );
        assert!(
            NotificationPreferences::default()
                .save_to(directory.path())
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"keep");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
