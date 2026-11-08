"""Check the exact patched release tree and security-critical source paths."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[3]
PACKAGING = ROOT / "packaging/updates/winsparkle"
SOURCE: Path


def text(name: str) -> str:
    return (SOURCE / name).read_text(encoding="utf-8-sig")


def body(source: str, name: str, following: str) -> str:
    return source.split(name, 1)[1].split(following, 1)[0]


class SourceChecks(unittest.TestCase):
    def test_exact_patched_release_bytes(self):
        manifest = json.loads((PACKAGING / "preimages.json").read_text(encoding="utf-8"))
        self.assertGreater(len(manifest), 0)
        for name, digests in manifest.items():
            with self.subTest(path=name):
                self.assertEqual(hashlib.sha256((SOURCE / name).read_bytes()).hexdigest(), digests["after_sha256"])

    def test_distinct_c_abi_and_lifetime_contract(self):
        header = text("include/winsparkle.h")
        self.assertIn("win_sparkle_get_hardening_version(void)", header)
        self.assertIn("const wchar_t *path, const unsigned char sha256[32]", header)
        self.assertIn("until process exit", header)
        api = text("src/dll_api.cpp")
        version = body(api, "win_sparkle_get_hardening_version(void)", "WIN_SPARKLE_API void")
        self.assertIn("return 1;", version)
        legacy = body(api, "win_sparkle_set_dsa_pub_pem(", "WIN_SPARKLE_API int")
        self.assertIn("return 0;", legacy)
        self.assertNotIn("SetDSAPubKeyPem", api)

    def test_single_private_buffer_verification_then_cng_hash(self):
        verifier = text("src/signatureverifier.cpp")
        self.assertIn("const std::vector<unsigned char> payload = ReadPayload(filename);", verifier)
        verify = verifier.index("if (ed25519_verify(")
        reject = verifier.index("throw BadSignatureException();", verify)
        digest = verifier.index("return HashVerifiedBuffer(payload);", reject)
        self.assertLess(verify, reject)
        self.assertLess(reject, digest)
        self.assertIn("payload.data(), payload.size()", verifier)
        self.assertIn("BCryptHashData(cng.hash", verifier)
        self.assertNotIn("openssl/", verifier)
        self.assertNotIn("DSA_verify", verifier)
        self.assertNotIn("VerifyDSASHA1", verifier)
        self.assertNotIn("CreateFileMapping", verifier)

    def test_event_dialog_and_callback_keep_bound_digest(self):
        ui = text("src/ui.cpp")
        self.assertIn("SHA256Digest verifiedDigest = {};", ui)
        self.assertIn("payload.verifiedDigest = digest;", ui)
        self.assertIn("payload.updateFile, payload.verifiedDigest, payload.downloadToken", ui)
        self.assertIn("m_verifiedDigest = digest;", ui)
        self.assertIn("token != m_downloadToken", ui)
        run = body(ui, "bool UpdateDialog::RunInstaller()", "void UpdateDialog::SetMessage")
        self.assertIn("m_hasVerifiedInstaller = false;", run)
        self.assertIn("m_updateFile.t_str(), m_verifiedDigest.data()", run)
        self.assertNotIn("SignatureVerifier", run)
        self.assertNotIn("ShellExecute", ui)
        self.assertNotIn("UserRunInstallerCallback", ui)
        controller = text("src/appcontroller.cpp")
        dispatch = body(controller, "bool ApplicationController::VerifiedInstallerCallback(", "} // namespace")
        self.assertIn("callback(filePath, digest) == 1", dispatch)
        self.assertIn("if (!callback || !filePath || !digest)", dispatch)
        self.assertIn("catch (...)", dispatch)

    def test_required_configuration_and_no_auto_install(self):
        api = text("src/dll_api.cpp")
        for start, end in [
            ("win_sparkle_init()", "win_sparkle_cleanup()"),
            ("win_sparkle_check_update_with_ui()", "win_sparkle_check_update_with_ui_and_install()"),
            ("win_sparkle_check_update_without_ui()", "win_sparkle_set_user_run_installer_callback("),
        ]:
            self.assertIn("RequireHardenedConfiguration();", body(api, start, end))
        self.assertIn("RequireHardenedConfiguration();", text("src/updatedownloader.cpp"))
        self.assertIn("RequireHardenedConfiguration();", text("src/updatechecker.cpp"))
        controller = text("src/appcontroller.cpp")
        required = body(controller, "void ApplicationController::RequireHardenedConfiguration()", "bool ApplicationController::VerifiedInstallerCallback")
        self.assertIn("VerifyEdDSAPubKey(Settings::GetEdDSAPubKey())", required)
        self.assertIn("if (!ms_cbVerifiedInstaller)", required)
        auto = body(api, "win_sparkle_check_update_with_ui_and_install()", "win_sparkle_check_update_without_ui()")
        self.assertNotIn("ManualAutoInstallUpdateChecker", auto)
        self.assertIn("win_sparkle_check_update_with_ui();", auto)
        available = body(text("src/ui.cpp"), "void UpdateDialog::StateUpdateAvailable(", "void UpdateDialog::StateDownloading()")
        self.assertIn("m_installAutomatically = false;", available)
        downloaded = body(text("src/ui.cpp"), "void UpdateDialog::StateUpdateDownloaded(", "void UpdateDialog::ShowReleaseNotes(")
        self.assertNotIn("OnRunInstaller", downloaded)
        self.assertIn("SHOW(m_closeButtonSizer)", downloaded)

    def test_fixed_local_name_before_verification(self):
        downloader = text("src/updatedownloader.cpp")
        self.assertIn('m_path = m_dir + L"\\\\payload.exe";', downloader)
        self.assertNotIn('m_dir + L"\\\\" + filename', downloader)
        download = text("src/download.cpp")
        self.assertIn('sink->SetFilename(L"payload.exe")', download)
        self.assertNotIn("HTTP_QUERY_CONTENT_DISPOSITION", download)
        self.assertNotIn("GetURLFileName", download)
        self.assertNotIn("Using unsigned updates", downloader)
        self.assertNotIn("HasDSAPubKeyPem", downloader)

    def test_inert_notes_and_no_browser_dependency(self):
        ui = text("src/ui.cpp")
        self.assertIn("new wxTextCtrl", ui)
        self.assertIn("wxTE_MULTILINE | wxTE_READONLY", ui)
        self.assertIn("ChangeValue(wxString::FromUTF8(info.Description.substr(0, 65536)))", ui)
        for executable in ["wxWebView", "wxLaunchDefaultBrowser", "LoadURL(", "SetPage("]:
            self.assertNotIn(executable, ui)
        config = text("3rdparty/config/wx/setup.h")
        for option in ["wxUSE_WEBVIEW", "wxUSE_WEBVIEW_IE", "wxUSE_WEBVIEW_EDGE"]:
            self.assertRegex(config, rf"#define {option} 0\b")
        wx = text("3rdparty/wx.vcxproj")
        self.assertNotRegex(wx, r'ClCompile Include="[^\"]*webview')
        self.assertNotIn("Microsoft.Web.WebView2", wx)
        self.assertNotIn("Microsoft.Web.WebView2", text("packages.config"))

    def test_https_policy_precedes_every_request(self):
        download = text("src/download.cpp")
        self.assertLess(download.index('CheckForInsecureURL(url, "update request")'), download.index("InternetOpenUrlA"))
        self.assertIn("urlc.nScheme != INTERNET_SCHEME_HTTPS", download)
        self.assertIn("INTERNET_FLAG_NO_AUTO_REDIRECT", download)
        self.assertIn("INTERNET_FLAG_NO_UI", download)
        self.assertIn('CheckForInsecureURL(next, "redirect")', download)
        self.assertIn("redirects > 5", download)
        self.assertIn("DownloadFileHTTPS(next, sink", download)
        self.assertRegex(text("src/utils.h"), r'throw std::runtime_error\("HTTPS is required')

    def test_synchronous_request_ownership_and_cancel_boundaries(self):
        download = text("src/download.cpp")
        for asynchronous in ["INTERNET_FLAG_ASYNC", "InternetSetStatusCallback", "DownloadCallbackContext",
                             "eventRequestComplete", "InternetReadFileEx", "INTERNET_BUFFERS", "ERROR_IO_PENDING"]:
            self.assertNotIn(asynchronous, download)
        self.assertIn("InetHandle(const InetHandle&) = delete;", download)
        self.assertIn("InetHandle& operator=(const InetHandle&) = delete;", download)
        timeouts = body(download, "void SetRequestTimeouts(", "} // anonymous namespace")
        self.assertIn("DWORD timeout = 10000;", timeouts)
        for option in ["CONNECT_TIMEOUT", "SEND_TIMEOUT", "RECEIVE_TIMEOUT"]:
            self.assertIn("INTERNET_OPTION_" + option, timeouts)
        self.assertIn("if (!InternetSetOptionW(inet, option", timeouts)
        self.assertIn("DWORD attempts = 1;", timeouts)
        self.assertIn("if (!InternetSetOptionW(inet, INTERNET_OPTION_CONNECT_RETRIES", timeouts)
        self.assertLess(download.index("SetRequestTimeouts(inet);"), download.index("InternetOpenUrlA"))
        request = body(download, "InetHandle conn(InternetOpenUrlA", "DWORD statusCode")
        self.assertIn("0 // no borrowed request context", request)
        self.assertLess(request.index("CheckDownloadCancelled(onThread);"), request.index("if (!conn)"))
        read = body(download, "char buffer[10240];", "void DownloadFile(")
        self.assertLess(read.index("CheckDownloadCancelled(onThread);"), read.index("InternetReadFile("))
        after_read = read.split("InternetReadFile(", 1)[1]
        self.assertLess(after_read.index("CheckDownloadCancelled(onThread);"), after_read.index("sink->Add("))
        redirect = body(download, 'CheckForInsecureURL(next, "redirect")', "if (statusCode != 200)")
        for operation in ["conn.Close();", "inet.Close();", "CheckDownloadCancelled(onThread);"]:
            self.assertLess(redirect.index(operation), redirect.index("DownloadFileHTTPS(next"))

    def test_actual_byte_limits_precede_append_write_and_parse(self):
        header = text("src/download.h")
        self.assertIn("MaxAppcastBytes = 1024 * 1024", header)
        self.assertIn("MaxPayloadBytes = 512 * 1024 * 1024", header)
        self.assertLess(header.index("this->data.size() > MaxAppcastBytes - len"), header.index("this->data.append("))
        sink = text("src/updatedownloader.cpp")
        self.assertLess(sink.index("m_downloaded > MaxPayloadBytes - len"), sink.index("fwrite(data, len"))
        verifier = text("src/signatureverifier.cpp")
        self.assertLess(verifier.index("static_cast<size_t>(size) > MaxPayloadBytes"), verifier.index("std::vector<unsigned char> payload("))
        parser = body(text("src/appcast.cpp"), "std::vector<Appcast> Appcast::Load(", "} // namespace")
        self.assertLess(parser.index("xml.size() > MaxAppcastBytes"), parser.index("XML_ParserCreateNS"))
        self.assertLess(parser.index("xml.size() > MaxAppcastBytes"), parser.index("static_cast<int>(xml.size())"))

    def test_periodic_failure_guard_keeps_normal_interval(self):
        scheduler = body(text("src/updatechecker.cpp"), "void PeriodicUpdateChecker::Run()", "void OneShotUpdateChecker::Run()")
        self.assertIn("catch (const TerminateThreadException&)", scheduler)
        self.assertIn("catch (...)\n                {", scheduler)
        self.assertIn("CheckShouldTerminate();", scheduler)
        self.assertIn("sleepTimeInSeconds = interval;", scheduler)
        self.assertIn("if (m_terminateEvent.WaitUntilSignaled(sleepTimeInSeconds * 1000))\n            return;", scheduler)

    def test_all_host_notifications_leave_configuration_lock(self):
        controller = text("src/appcontroller.cpp")
        for name in ["NotifyUpdateError", "NotifyUpdateFound", "NotifyUpdateNotFound", "NotifyUpdateCancelled",
                     "NotifyUpdateSkipped", "NotifyUpdatePostponed", "NotifyUpdateDismissed", "RequestShutdown"]:
            dispatch = body(controller, "void ApplicationController::" + name + "()", "\n}\n")
            self.assertRegex(dispatch, r"callback = ms_cb\w+;\n    }\n    if \(callback\)\n        callback\(\);")

    def test_active_offer_is_frozen_and_worker_events_are_tagged(self):
        ui = text("src/ui.cpp")
        for start, end in [("void UpdateDialog::StateCheckingUpdates()", "void UpdateDialog::StateNoUpdateFound"),
                           ("void UpdateDialog::StateNoUpdateFound(", "void UpdateDialog::StateUpdateError"),
                           ("void UpdateDialog::StateUpdateAvailable(", "void UpdateDialog::StateDownloading")]:
            self.assertIn("if (HasActiveUpdate())\n        return;", body(ui, start, end))
        install = body(ui, "void UpdateDialog::OnInstall(", "void UpdateDialog::OnRunInstaller(")
        self.assertLess(install.index("m_downloadToken = m_downloader->GetToken();"), install.index("m_downloader->Start();"))
        for start, end in [("void UpdateDialog::DownloadProgress(", "void UpdateDialog::StateUpdateDownloaded("),
                           ("void UpdateDialog::StateUpdateError(", "void UpdateDialog::StateUpdateAvailable("),
                           ("void UpdateDialog::StateUpdateDownloaded(", "void UpdateDialog::ShowReleaseNotes(")]:
            self.assertIn("!m_downloader || token != m_downloadToken", body(ui, start, end))
        downloader = text("src/updatedownloader.cpp")
        self.assertIn("NotifyDownloadProgress(m_downloaded, m_total, m_token)", downloader)
        self.assertIn("NotifyUpdateError(Err_Generic, m_token)", downloader)
        self.assertIn("NotifyUpdateError(Err_BadSignature, m_token)", downloader)
        self.assertIn("payload.error, payload.downloadToken", ui)
        self.assertIn("payload.sizeDownloaded, payload.sizeTotal, payload.downloadToken", ui)

    def test_active_builds_exclude_legacy_crypto(self):
        ns = {"m": "http://schemas.microsoft.com/developer/msbuild/2003"}
        project = ET.parse(SOURCE / "WinSparkle.vcxproj").getroot()
        refs = [x.attrib["Include"] for x in project.findall(".//m:ProjectReference", ns)]
        self.assertEqual(set(refs), {r"3rdparty\expat.vcxproj", r"3rdparty\wx.vcxproj", r"3rdparty\ed25519.vcxproj"})
        self.assertNotIn("WinSparkle_libcrypto", text("WinSparkle.sln"))
        self.assertNotIn("libcrypto.bkl", text("3rdparty/dependencies.bkl"))
        self.assertNotIn("WinSparkle_libcrypto", text("WinSparkle.bkl"))
        cmake = text("cmake/CMakeLists.txt")
        self.assertNotIn("add_subdirectory(crypto)", cmake)
        self.assertNotIn("TARGET_OBJECTS:crypto", cmake)
        self.assertIn("TARGET_OBJECTS:ed25519", cmake)
        self.assertIn("crypt32 bcrypt", cmake)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    args = parser.parse_args()
    SOURCE = args.source.resolve(strict=True)
    unittest.main(argv=[__file__], verbosity=2)
