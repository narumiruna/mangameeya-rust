// Native SDK unit checks. Only loopback TLS failures use real networking.
// No certificate trust is changed, no TLS checks are bypassed and no installer runs.
#include <winsock2.h>
#include <wininet.h>
#include <wx/init.h>
#include <wx/textctrl.h>
#include <windows.h>
#include <winioctl.h>
#include <bcrypt.h>
#include <wincrypt.h>
#include <ed25519.h>
#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cstdio>
#include <functional>
#include <iostream>
#include <map>
#include <stdexcept>
#include <thread>
#include <vector>

#pragma comment(lib, "ws2_32.lib")

// Compile the real event payload and fixed-name download sink in this unit.
#include "ui.cpp"
// Capture worker messages without starting a second wx UI thread in this unit.
// Production UI state methods below still run unchanged on hidden real controls.
class FixtureDownloadUI
{
public:
    static void NotifyDownloadProgress(size_t, size_t, std::shared_ptr<const int>);
    static void NotifyUpdateDownloaded(const std::wstring&, const winsparkle::SHA256Digest&, std::shared_ptr<const int>);
    static void NotifyUpdateError(winsparkle::ErrorCode, std::shared_ptr<const int>);
};
#define UI FixtureDownloadUI
#include "updatedownloader.cpp"
#undef UI

using namespace winsparkle;

static void Check(bool ok, const char* message)
{
    if (!ok) throw std::runtime_error(message);
}
static void Reject(const std::function<void()>& action, const char* reason = nullptr)
{
    bool failed = false;
    try { action(); }
    catch (const std::exception& error)
    {
        failed = true;
        if (reason)
            Check(std::string(error.what()).find(reason) != std::string::npos, "Wrong rejection stage");
    }
    Check(failed, "Unexpected acceptance");
}
static Event downloadNotification;
static CriticalSection downloadNotificationLock;
static EventPayload lastDownloadNotification;
void FixtureDownloadUI::NotifyDownloadProgress(size_t downloaded, size_t total, std::shared_ptr<const int> token)
{
    CriticalSectionLocker lock(downloadNotificationLock);
    lastDownloadNotification = {};
    lastDownloadNotification.sizeDownloaded = downloaded;
    lastDownloadNotification.sizeTotal = total;
    lastDownloadNotification.downloadToken = token;
    downloadNotification.Signal();
}
void FixtureDownloadUI::NotifyUpdateDownloaded(const std::wstring& path, const SHA256Digest& digest, std::shared_ptr<const int> token)
{
    CriticalSectionLocker lock(downloadNotificationLock);
    lastDownloadNotification = {};
    lastDownloadNotification.updateFile = path;
    lastDownloadNotification.verifiedDigest = digest;
    lastDownloadNotification.downloadToken = token;
    downloadNotification.Signal();
}
void FixtureDownloadUI::NotifyUpdateError(ErrorCode error, std::shared_ptr<const int> token)
{
    CriticalSectionLocker lock(downloadNotificationLock);
    lastDownloadNotification = {};
    lastDownloadNotification.error = error;
    lastDownloadNotification.downloadToken = token;
    downloadNotification.Signal();
}
static int __cdecl ConfigRead(const char* name, wchar_t* buffer, size_t length, void* context)
{
    auto& values = *static_cast<std::map<std::string, std::wstring>*>(context);
    auto entry = values.find(name);
    if (entry == values.end() || entry->second.size() >= length) return 0;
    wcscpy_s(buffer, length, entry->second.c_str());
    return 1;
}
static void __cdecl ConfigWrite(const char* name, const wchar_t* value, void* context)
{
    (*static_cast<std::map<std::string, std::wstring>*>(context))[name] = value;
}
static void __cdecl ConfigDelete(const char* name, void* context)
{
    static_cast<std::map<std::string, std::wstring>*>(context)->erase(name);
}

static std::string Base64(const unsigned char* bytes, DWORD size)
{
    DWORD length = 0;
    Check(CryptBinaryToStringA(bytes, size, CRYPT_STRING_BASE64 | CRYPT_STRING_NOCRLF, NULL, &length) != 0, "base64 length");
    std::string encoded(length, '\0');
    Check(CryptBinaryToStringA(bytes, size, CRYPT_STRING_BASE64 | CRYPT_STRING_NOCRLF, &encoded[0], &length) != 0, "base64 encode");
    encoded.resize(strlen(encoded.c_str()));
    return encoded;
}
static void WriteFixture(const std::wstring& file, const std::vector<unsigned char>& data)
{
    FILE* f = _wfopen(file.c_str(), L"wb");
    Check(f != NULL, "open fixture");
    const auto written = fwrite(data.data(), 1, data.size(), f);
    const auto closed = fclose(f);
    Check(written == data.size() && closed == 0, "write fixture");
}
static SHA256Digest received;
static int callbackResult = 1;
static int callbackCount = 0;
static std::wstring receivedPath;
static int __cdecl Callback(const wchar_t* path, const unsigned char digest[32])
{
    ++callbackCount;
    receivedPath = path;
    std::copy(digest, digest + 32, received.begin());
    // Callback dispatch must not hold the SDK callback lock.
    std::thread setter([] { ApplicationController::SetVerifiedInstallerCallback(Callback); });
    setter.join();
    return callbackResult;
}
static int __cdecl ThrowingCallback(const wchar_t*, const unsigned char*)
{
    throw std::runtime_error("fixture exception");
}
class FixtureThread : public Thread
{
public:
    FixtureThread() : Thread("SDK gate fixture") {}
    void RequestCancel() { m_terminateEvent.Signal(); }
    bool IsCancelled(const std::function<void()>& action)
    {
        try { action(); }
        catch (const TerminateThreadException&) { return true; }
        return false;
    }
protected:
    void Run() override { SignalReady(); }
    bool IsJoinable() const override { return true; }
};

// Compile a second copy of the real download implementation with a deterministic
// WinINet seam. The normal DownloadFile below still links the unmodified WinINet
// calls. This seam covers synchronous success without a certificate or HTTP bypass.
namespace scripted
{
enum class Result { Success, OpenError, ReadError, SinkError, TimeoutError, CancelOpen, CancelRead, Redirect };
static Result result;
static FixtureThread* cancelThread;
static bool session, request, badOwnership;
static unsigned opens, reads, closedRequests, closedSessions, timeoutOptions;
static HINTERNET Session() { return reinterpret_cast<HINTERNET>(1); }
static HINTERNET Request() { return reinterpret_cast<HINTERNET>(2); }
static HINTERNET WINAPI Open(LPCWSTR, DWORD, LPCWSTR, LPCWSTR, DWORD flags)
{
    badOwnership |= flags != 0 || session || request;
    session = true;
    return Session();
}
static BOOL WINAPI Option(HINTERNET handle, DWORD option, LPVOID value, DWORD length)
{
    badOwnership |= handle != Session() || !session || length != sizeof(DWORD);
    if (option == INTERNET_OPTION_CONNECT_TIMEOUT || option == INTERNET_OPTION_SEND_TIMEOUT ||
        option == INTERNET_OPTION_RECEIVE_TIMEOUT || option == INTERNET_OPTION_CONNECT_RETRIES)
    {
        ++timeoutOptions;
        badOwnership |= *static_cast<DWORD*>(value) != (option == INTERNET_OPTION_CONNECT_RETRIES ? 1u : 10000u);
        if (result == Result::TimeoutError) { SetLastError(ERROR_INVALID_PARAMETER); return FALSE; }
    }
    return TRUE;
}
static HINTERNET WINAPI OpenURL(HINTERNET handle, LPCSTR, LPCSTR, DWORD, DWORD flags, DWORD_PTR context)
{
    badOwnership |= handle != Session() || !session || request || context != 0 ||
                    !(flags & INTERNET_FLAG_NO_AUTO_REDIRECT) || timeoutOptions != 4 * (opens + 1);
    ++opens;
    if (result == Result::CancelOpen) cancelThread->RequestCancel();
    if (result == Result::OpenError) { SetLastError(ERROR_INTERNET_CANNOT_CONNECT); return NULL; }
    request = true;
    return Request(); // No callback is sent for this synchronous success.
}
static BOOL WINAPI Header(HINTERNET handle, DWORD query, LPVOID value, LPDWORD length, LPDWORD)
{
    badOwnership |= handle != Request() || !request;
    if (query == (HTTP_QUERY_STATUS_CODE | HTTP_QUERY_FLAG_NUMBER))
    {
        *static_cast<DWORD*>(value) = result == Result::Redirect && opens == 1 ? 302 : 200;
        return TRUE;
    }
    if (query == HTTP_QUERY_LOCATION)
    {
        const char location[] = "/redirected";
        memcpy(value, location, sizeof(location));
        *length = sizeof(location) - 1;
        return TRUE;
    }
    return FALSE; // Content length is optional.
}
static BOOL WINAPI Read(HINTERNET handle, LPVOID buffer, DWORD size, LPDWORD length)
{
    badOwnership |= handle != Request() || !request || size < 3;
    ++reads;
    if (result == Result::CancelRead) cancelThread->RequestCancel();
    if (result == Result::ReadError) { SetLastError(ERROR_INTERNET_CONNECTION_ABORTED); return FALSE; }
    *length = reads == 1 ? 3 : 0;
    if (*length) memcpy(buffer, "abc", 3);
    return TRUE;
}
static BOOL WINAPI Close(HINTERNET handle)
{
    if (handle == Request()) { badOwnership |= !request; request = false; ++closedRequests; }
    else { badOwnership |= handle != Session() || !session || request; session = false; ++closedSessions; }
    return TRUE;
}
}
#define InternetOpenW scripted::Open
#define InternetOpenUrlA scripted::OpenURL
#define InternetSetOptionW scripted::Option
#define InternetReadFile scripted::Read
#define InternetCloseHandle scripted::Close
#define HttpQueryInfoA scripted::Header
#define DownloadFile ScriptedDownloadFile
#include "download.cpp"
#undef DownloadFile
#undef HttpQueryInfoA
#undef InternetCloseHandle
#undef InternetReadFile
#undef InternetSetOptionW
#undef InternetOpenUrlA
#undef InternetOpenW

static void TestSynchronousRequestOwnership()
{
    using scripted::Result;
    for (auto result : {Result::Success, Result::OpenError, Result::ReadError, Result::SinkError, Result::TimeoutError,
                        Result::CancelOpen, Result::CancelRead, Result::Redirect})
    {
        FixtureThread thread;
        thread.Start(); thread.Join();
        scripted::result = result;
        scripted::cancelThread = &thread;
        scripted::session = scripted::request = scripted::badOwnership = false;
        scripted::opens = scripted::reads = scripted::closedRequests = scripted::closedSessions = scripted::timeoutOptions = 0;
        struct Sink : StringDownloadSink
        {
            void Add(const void* bytes, size_t size) override
            {
                if (scripted::result == Result::SinkError) throw std::runtime_error("fixture sink error");
                StringDownloadSink::Add(bytes, size);
            }
        } sink;
        auto download = [&] { ScriptedDownloadFile("https://example.invalid/test", &sink, &thread, "", 0); };
        if (result == Result::CancelOpen || result == Result::CancelRead)
        {
            Check(thread.IsCancelled(download), "cancel must unwind after the synchronous API returns");
            Check(sink.data.empty(), "cancelled bytes must not reach the sink");
        }
        else if (result == Result::OpenError || result == Result::ReadError || result == Result::SinkError || result == Result::TimeoutError)
            Reject(download);
        else
        {
            download();
            Check(sink.data == "abc" && scripted::reads == 2, "synchronous open/read/EOF without any callback");
        }
        const unsigned sessions = result == Result::Redirect ? 2 : 1;
        const unsigned requests = result == Result::OpenError || result == Result::TimeoutError ? 0 : sessions;
        Check(!scripted::session && !scripted::request && !scripted::badOwnership, "exclusive synchronous handle ownership");
        Check(scripted::closedSessions == sessions && scripted::closedRequests == requests, "all request handles closed exactly once");
    }
}

struct FixtureSocket
{
    SOCKET value = INVALID_SOCKET;
    ~FixtureSocket() { if (value != INVALID_SOCKET) closesocket(value); }
};
class LoopbackTLSPeer
{
public:
    explicit LoopbackTLSPeer(bool stall) : m_stall(stall)
    {
        m_listener.value = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
        Check(m_listener.value != INVALID_SOCKET, "loopback socket");
        sockaddr_in address = {};
        address.sin_family = AF_INET;
        address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        Check(bind(m_listener.value, reinterpret_cast<sockaddr*>(&address), sizeof(address)) == 0, "bind loopback");
        Check(listen(m_listener.value, 1) == 0, "listen loopback");
        int size = sizeof(address);
        Check(getsockname(m_listener.value, reinterpret_cast<sockaddr*>(&address), &size) == 0, "loopback port");
        m_url = "https://127.0.0.1:" + std::to_string(ntohs(address.sin_port)) + "/fixture";
        m_server = std::thread([this] { Serve(); });
    }
    ~LoopbackTLSPeer() { Release(); m_server.join(); }
    const std::string& URL() const { return m_url; }
    bool WaitForTLS() { return m_hello.WaitUntilSignaled(5000) && m_sawTLS; }
    void Release() { m_release.Signal(); }
private:
    void Serve()
    {
        fd_set readable;
        FD_ZERO(&readable); FD_SET(m_listener.value, &readable);
        timeval wait = {5, 0};
        if (select(0, &readable, NULL, NULL, &wait) != 1) return;
        FixtureSocket client;
        client.value = accept(m_listener.value, NULL, NULL);
        if (client.value == INVALID_SOCKET) return;
        // WinINet can retry a failed TLS negotiation. Do not leave a listening
        // socket with an unserviced accept queue after this single fixture.
        closesocket(m_listener.value);
        m_listener.value = INVALID_SOCKET;
        DWORD timeout = 5000;
        if (setsockopt(client.value, SOL_SOCKET, SO_RCVTIMEO, reinterpret_cast<const char*>(&timeout), sizeof(timeout)) != 0) return;
        unsigned char header[5];
        int count = 0;
        while (count < sizeof(header))
        {
            const int got = recv(client.value, reinterpret_cast<char*>(header) + count, sizeof(header) - count, 0);
            if (got <= 0) return;
            count += got;
        }
        m_sawTLS = header[0] == 0x16 && header[1] == 3; // TLS ClientHello, never plaintext HTTP.
        m_hello.Signal();
        if (m_stall) m_release.WaitUntilSignaled(25000);
        // Fatal handshake_failure, without a certificate or test private key.
        const char alert[] = {0x15, 3, 3, 0, 2, 2, 40};
        send(client.value, alert, sizeof(alert), 0);
        shutdown(client.value, SD_BOTH);
    }
    bool m_stall;
    std::atomic<bool> m_sawTLS{false};
    FixtureSocket m_listener;
    Event m_hello, m_release;
    std::string m_url;
    std::thread m_server;
};
class RequestFixtureThread : public Thread
{
public:
    explicit RequestFixtureThread(const std::string& url) : Thread("WinINet lifetime fixture"), m_url(url) {}
    ~RequestFixtureThread() { TerminateAndJoin(); }
    void RequestCancel() { m_terminateEvent.Signal(); }
    bool Wait(unsigned timeout) { return m_done.WaitUntilSignaled(timeout); }
    std::atomic<bool> cancelled{false}, failed{false}, succeeded{false};
    std::atomic<unsigned> sinkCalls{0};
protected:
    void Run() override
    {
        struct Sink : IDownloadSink
        {
            std::atomic<unsigned>& calls;
            explicit Sink(std::atomic<unsigned>& count) : calls(count) {}
            void SetLength(size_t) override { ++calls; }
            void SetFilename(const std::wstring&) override { ++calls; }
            void Add(const void*, size_t) override { ++calls; }
        } sink(sinkCalls);
        SignalReady();
        try { DownloadFile(m_url, &sink, this); succeeded = true; }
        catch (const TerminateThreadException&) { cancelled = true; }
        catch (const std::exception&) { failed = true; }
        m_done.Signal();
    }
    bool IsJoinable() const override { return true; }
private:
    std::string m_url;
    Event m_done;
};
static void TestLoopbackRequestOwnership()
{
    WSADATA data;
    Check(WSAStartup(MAKEWORD(2, 2), &data) == 0, "Winsock startup");
    struct Cleanup { ~Cleanup() { WSACleanup(); } } cleanup;
    for (unsigned attempt = 0; attempt < 5; ++attempt)
    {
        const bool timeout = attempt == 1;
        const bool cancel = attempt > 1;
        std::cerr << "native: loopback attempt " << attempt << (cancel ? " cancel\n" : timeout ? " timeout\n" : " error\n");
        LoopbackTLSPeer peer(cancel || timeout);
        const auto started = std::chrono::steady_clock::now();
        auto request = std::make_unique<RequestFixtureThread>(peer.URL());
        request->Start();
        Check(peer.WaitForTLS(), "request must reach loopback TLS, without proxy or trust changes");
        if (cancel)
        {
            request->RequestCancel();
            Check(!request->Wait(100), "cancel must retain ownership while WinINet is still using the request");
            peer.Release();
        }
        Check(request->Wait(timeout ? 22000 : 5000), "loopback timeout/error/cancel did not finish");
        request->Join();
        std::cerr << "native: loopback completion ms "
                  << std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - started).count() << '\n';
        Check(!request->succeeded && request->sinkCalls == 0, "TLS failure cannot deliver payload bytes");
        Check(cancel ? request->cancelled && !request->failed : request->failed && !request->cancelled,
              "loopback cancellation/error outcome");
        // Destroy the joined owner before the peer. There is no callback state
        // or borrowed read buffer left for delayed WinINet activity to access.
        request.reset();
    }
}

static void TestByteBounds(const std::wstring& directory, const std::string& signature)
{
    const std::string bytes(MaxAppcastBytes, 'x');
    StringDownloadSink feed;
    feed.SetLength(1); // A false or missing length must not bypass actual-byte limits.
    feed.Add(bytes.data(), bytes.size());
    Reject([&] { feed.Add("x", 1); }, "Appcast exceeds size limit");
    Reject([&] { feed.Add("x", static_cast<size_t>(-1)); }, "Appcast exceeds size limit");
    Reject([&] { feed.SetLength(MaxAppcastBytes + 1); }, "Appcast exceeds size limit");
    Check(feed.data.size() == MaxAppcastBytes, "oversized append changed feed bytes");
    Reject([&] { Appcast::Load(bytes + "x"); }, "Appcast exceeds size limit");
    std::string boundedXML = "<rss><channel></channel></rss>";
    boundedXML.resize(MaxAppcastBytes, ' ');
    Check(Appcast::Load(boundedXML).empty(), "bounded XML parser input");

    FixtureThread thread;
    thread.Start(); thread.Join();
    {
        UpdateDownloadSink sink(thread, directory, std::make_shared<const int>(0));
        sink.SetLength(1);
        Reject([&] { sink.SetLength(MaxPayloadBytes + 1); }, "Update payload exceeds size limit");
        sink.SetFilename(L"ignored");
        // Seed the counter at the boundary; do not write a 512 MiB test file.
        sink.m_downloaded = MaxPayloadBytes - 1;
        sink.Add("x", 1);
        Reject([&] { sink.Add("x", 1); }, "Update payload exceeds size limit");
        Reject([&] { sink.Add("x", static_cast<size_t>(-1)); }, "Update payload exceeds size limit");
        Check(ftell(sink.m_file) == 1, "oversized chunk was written before rejection");
    }
    Check(DeleteFileW((directory + L"\\payload.exe").c_str()) != 0, "remove byte-limit sink fixture");

    const std::wstring path = directory + L"\\oversized-fixture.bin";
    HANDLE file = CreateFileW(path.c_str(), GENERIC_WRITE, 0, NULL, CREATE_NEW, FILE_ATTRIBUTE_TEMPORARY, NULL);
    Check(file != INVALID_HANDLE_VALUE, "create sparse oversized fixture");
    DWORD returned;
    const BOOL sparse = DeviceIoControl(file, FSCTL_SET_SPARSE, NULL, 0, NULL, 0, &returned, NULL);
    LARGE_INTEGER size;
    size.QuadPart = MaxPayloadBytes + 1;
    const BOOL sized = SetFilePointerEx(file, size, NULL, FILE_BEGIN) && SetEndOfFile(file);
    const BOOL closed = CloseHandle(file);
    struct Remove { std::wstring path; ~Remove() { DeleteFileW(path.c_str()); } } remove{path};
    Check(sparse && sized && closed, "prepare sparse oversized fixture");
    Reject([&] { SignatureVerifier::VerifyEdDSASignatureValid(path, signature); }, "Invalid update file size");
}

class PeriodicFixture : public PeriodicUpdateChecker
{
public:
    explicit PeriodicFixture(bool cancel) : m_cancel(cancel) {}
    ~PeriodicFixture() { TerminateAndJoin(); }
    Event attempted;
    std::atomic<unsigned> attempts{0};
    bool Alive() const { return WaitForSingleObject(m_handle, 0) == WAIT_TIMEOUT; }
    bool Finished() const { return WaitForSingleObject(m_handle, 5000) == WAIT_OBJECT_0; }
protected:
    void PerformUpdateCheck() override
    {
        ++attempts;
        attempted.Signal();
        if (m_cancel)
        {
            m_terminateEvent.Signal();
            CheckShouldTerminate();
        }
        throw std::runtime_error("fixture offline/XML failure");
    }
    bool IsJoinable() const override { return true; }
private:
    bool m_cancel;
};
static void TestPeriodicFailureRecovery()
{
    Settings::WriteConfigValue("CheckForUpdates", true);
    Settings::WriteConfigValue("LastCheckTime", 0);
    win_sparkle_set_update_check_interval(3600);
    {
        PeriodicFixture checker(false);
        checker.Start();
        Check(checker.attempted.WaitUntilSignaled(5000), "periodic fixture first attempt");
        Sleep(100);
        Check(checker.Alive() && checker.attempts == 1, "periodic failure killed scheduler or caused a busy retry");
        checker.TerminateAndJoin();
    }
    {
        PeriodicFixture checker(true);
        checker.Start();
        Check(checker.attempted.WaitUntilSignaled(5000), "periodic cancellation attempt");
        Check(checker.Finished() && checker.attempts == 1, "periodic guard swallowed cancellation");
    }
    Settings::WriteConfigValue("CheckForUpdates", false);
}
static unsigned notificationCount = 0;
static void __cdecl NotificationCallback();
static int __cdecl ReadyCallback();
static void SetNotifications()
{
    ApplicationController::SetErrorCallback(NotificationCallback);
    ApplicationController::SetCanShutdownCallback(ReadyCallback);
    ApplicationController::SetShutdownRequestCallback(NotificationCallback);
    ApplicationController::SetDidFindUpdateCallback(NotificationCallback);
    ApplicationController::SetDidNotFindUpdateCallback(NotificationCallback);
    ApplicationController::SetUpdateCancelledCallback(NotificationCallback);
    ApplicationController::SetUpdateSkippedCallback(NotificationCallback);
    ApplicationController::SetUpdatePostponedCallback(NotificationCallback);
    ApplicationController::SetUpdateDismissedCallback(NotificationCallback);
}
static void __cdecl NotificationCallback()
{
    ++notificationCount;
    std::thread setter(SetNotifications);
    setter.join(); // Deadlocks if the SDK still holds its callback lock.
}
static int __cdecl ReadyCallback() { NotificationCallback(); return 1; }
static void TestNotificationLocks()
{
    SetNotifications();
    for (auto notify : {ApplicationController::NotifyUpdateError, ApplicationController::NotifyUpdateFound,
                        ApplicationController::NotifyUpdateNotFound, ApplicationController::NotifyUpdateCancelled,
                        ApplicationController::NotifyUpdateSkipped, ApplicationController::NotifyUpdatePostponed,
                        ApplicationController::NotifyUpdateDismissed, ApplicationController::RequestShutdown})
    {
        const auto before = notificationCount;
        notify();
        Check(notificationCount == before + 1, "notification callback did not run");
    }
    const auto before = notificationCount;
    Check(ApplicationController::IsReadyToShutdown() && notificationCount == before + 1, "shutdown callback lock");
}
static void TestActiveOfferIdentity(const std::wstring& path, const SHA256Digest& digest)
{
    WSADATA data;
    Check(WSAStartup(MAKEWORD(2, 2), &data) == 0, "UI loopback Winsock startup");
    struct Cleanup { ~Cleanup() { WSACleanup(); } } cleanup;
    for (bool ready : {false, true})
    {
        while (downloadNotification.CheckIfSignaled()) {}
        LoopbackTLSPeer peer(true);
        UpdateDialog dialog;
        Appcast original;
        original.Version = "2.0.0";
        original.Description = "Original offer";
        original.enclosure.DownloadURL = peer.URL();
        dialog.StateUpdateAvailable(original, false);
        wxTextCtrl* notes = nullptr;
        wxGauge* progress = nullptr;
        for (auto child : dialog.GetChildren())
        {
            if (auto text = wxDynamicCast(child, wxTextCtrl)) notes = text;
            if (auto gauge = wxDynamicCast(child, wxGauge)) progress = gauge;
        }
        Check(notes && progress, "active-offer controls");
        wxCommandEvent install(wxEVT_COMMAND_BUTTON_CLICKED, ID_INSTALL);
        dialog.ProcessWindowEvent(install);
        Check(peer.WaitForTLS(), "active-offer downloader reached TLS");
        Appcast unrelated = original;
        unrelated.Version = "3.0.0";
        unrelated.Description = "Unrelated offer";
        dialog.StateUpdateAvailable(unrelated, false);
        dialog.StateCheckingUpdates();
        dialog.StateNoUpdateFound(false);
        dialog.StateUpdateError(Err_Generic);
        Check(notes->GetValue() == "Original offer", "checker replaced active offer");
        peer.Release();
        Check(downloadNotification.WaitUntilSignaled(5000), "tagged downloader error");
        EventPayload notification;
        {
            CriticalSectionLocker lock(downloadNotificationLock);
            notification = lastDownloadNotification;
        }
        Check(bool(notification.downloadToken), "download error lost session identity");
        const auto stale = std::make_shared<const int>(0);
        dialog.StateUpdateError(Err_Generic, stale);
        dialog.StateUpdateDownloaded(path, digest, stale);
        dialog.DownloadProgress(99, 100, stale);
        dialog.DownloadProgress(5, 100, notification.downloadToken);
        Check(progress->GetValue() == 5, "stale events changed active download or valid progress was dropped");
        if (ready)
        {
            // Inject only the already verified inert identity to exercise the
            // real confirmation state. This does not claim a positive TLS transfer.
            dialog.StateUpdateDownloaded(path, digest, notification.downloadToken);
            dialog.StateUpdateAvailable(unrelated, false);
            dialog.StateCheckingUpdates();
            dialog.StateNoUpdateFound(false);
            dialog.StateUpdateError(Err_Generic);
            dialog.StateUpdateError(Err_Generic, notification.downloadToken);
            dialog.DownloadProgress(99, 100, notification.downloadToken);
            Check(notes->GetValue() == "Original offer" && progress->GetRange() == 1 && progress->GetValue() == 1,
                  "checker or late download event replaced verified confirmation");
        }
        else
        {
            dialog.StateUpdateError(notification.error, notification.downloadToken);
            dialog.StateUpdateAvailable(unrelated, false);
            Check(notes->GetValue() == "Unrelated offer", "current download error did not release the active session");
        }
        UpdateDownloader::CleanLeftovers();
    }
}

int main()
{
    try
    {
        std::cerr << "native: configuration\n";
        std::map<std::string, std::wstring> configuration;
        win_sparkle_config_methods_t methods = {ConfigRead, ConfigWrite, ConfigDelete, &configuration};
        win_sparkle_set_config_methods(&methods); // Never read or write the user's registry.
        // Fail before SDK initialization/registry/worker/network activity.
        Check(win_sparkle_get_hardening_version() == 1, "capability version");
        Reject([] { ApplicationController::RequireHardenedConfiguration(); });
        Check(win_sparkle_set_dsa_pub_pem("unused") == 0, "DSA must fail closed");
        Check(win_sparkle_set_eddsa_public_key(NULL) == 0, "NULL key");
        Check(win_sparkle_set_eddsa_public_key("bad") == 0, "malformed key");

        std::array<unsigned char, 32> seed, pub;
        std::array<unsigned char, 64> secret, signature;
        Check(BCryptGenRandom(NULL, seed.data(), static_cast<ULONG>(seed.size()), BCRYPT_USE_SYSTEM_PREFERRED_RNG) >= 0, "random fixture seed");
        ed25519_create_keypair(pub.data(), secret.data(), seed.data());
        const auto key = Base64(pub.data(), static_cast<DWORD>(pub.size()));
        Check(win_sparkle_set_eddsa_public_key(key.c_str()) == 1, "valid key");
        Reject([] { ApplicationController::RequireHardenedConfiguration(); });
        win_sparkle_set_verified_installer_callback(Callback);
        ApplicationController::RequireHardenedConfiguration();

        for (const auto* url : {"http://127.0.0.1:9/", "file:///C:/payload.exe", "ftp://example.invalid/a", "https://user:pass@example.invalid/a"})
        {
            StringDownloadSink sink;
            const char* reason = std::string(url).find("https://") == 0
                ? "Only HTTPS without URL credentials" : "HTTPS is required";
            Reject([&] { DownloadFile(url, &sink, NULL); }, reason);
        }
        std::cerr << "native: synchronous request ownership\n";
        TestSynchronousRequestOwnership();
        std::cerr << "native: loopback TLS error/cancel ownership\n";
        TestLoopbackRequestOwnership();
        std::cerr << "native: periodic check failure recovery and cancellation\n";
        TestPeriodicFailureRecovery();
        std::cerr << "native: notification callback locks\n";
        TestNotificationLocks();
        std::cerr << "native: signatures\n";
        const std::wstring file = L"verified-fixture.txt";
        const std::vector<unsigned char> payload = {'a', 'b', 'c'};
        WriteFixture(file, payload);
        ed25519_sign(signature.data(), payload.data(), payload.size(), pub.data(), secret.data());
        const auto sig = Base64(signature.data(), static_cast<DWORD>(signature.size()));
        const auto digest = SignatureVerifier::VerifyEdDSASignatureValid(file, sig);
        const SHA256Digest expected = {0xba,0x78,0x16,0xbf,0x8f,0x01,0xcf,0xea,0x41,0x41,0x40,0xde,0x5d,0xae,0x22,0x23,
                                       0xb0,0x03,0x61,0xa3,0x96,0x17,0x7a,0x9c,0xb4,0x10,0xff,0x61,0xf2,0x00,0x15,0xad};
        Check(digest == expected, "verified SHA256 abc known answer");
        Reject([&] { SignatureVerifier::VerifyEdDSASignatureValid(file, ""); });
        Reject([&] { SignatureVerifier::VerifyEdDSASignatureValid(file, sig + " "); });
        auto wrongSig = sig; wrongSig[0] = wrongSig[0] == 'A' ? 'B' : 'A';
        Reject([&] { SignatureVerifier::VerifyEdDSASignatureValid(file, wrongSig); });
        std::array<unsigned char, 32> otherSeed = seed; otherSeed[0] ^= 1;
        std::array<unsigned char, 32> otherPub;
        std::array<unsigned char, 64> otherSecret;
        ed25519_create_keypair(otherPub.data(), otherSecret.data(), otherSeed.data());
        Settings::SetEdDSAPubKey(Base64(otherPub.data(), static_cast<DWORD>(otherPub.size())));
        Reject([&] { SignatureVerifier::VerifyEdDSASignatureValid(file, sig); });
        Settings::SetEdDSAPubKey(key);
        WriteFixture(file, {});
        Reject([&] { SignatureVerifier::VerifyEdDSASignatureValid(file, sig); });
        WriteFixture(file, {'a', 'b'});
        Reject([&] { SignatureVerifier::VerifyEdDSASignatureValid(file, sig); });
        WriteFixture(file, {'x', 'y', 'z'});
        Reject([&] { SignatureVerifier::VerifyEdDSASignatureValid(file, sig); });

        // The queued event owns a copy, not a pointer to worker stack storage.
        std::cerr << "native: event and callback\n";
        EventPayload queued;
        queued.updateFile = file;
        queued.verifiedDigest = digest;
        queued.downloadToken = std::make_shared<const int>(0);
        wxThreadEvent event;
        event.SetPayload(queued);
        queued.updateFile.clear(); queued.verifiedDigest.fill(0); queued.downloadToken.reset();
        const auto delivered = event.GetPayload<EventPayload>();
        Check(delivered.updateFile == file && delivered.verifiedDigest == expected && delivered.downloadToken, "event owns verified identity");
        // Path bytes were replaced AFTER signature verification. The callback
        // must still receive the old trusted digest, not a hash of xyz.
        Check(ApplicationController::VerifiedInstallerCallback(delivered.updateFile.c_str(), delivered.verifiedDigest.data()), "callback handled");
        Check(received == expected && receivedPath == file && callbackCount == 1, "bound digest survives replacement");
        for (int result : {0, -1, 2})
        {
            callbackResult = result;
            Check(!ApplicationController::VerifiedInstallerCallback(file.c_str(), digest.data()), "callback failure cannot fall back");
        }
        win_sparkle_set_verified_installer_callback(ThrowingCallback);
        Check(!ApplicationController::VerifiedInstallerCallback(file.c_str(), digest.data()), "exception fails closed");
        win_sparkle_set_verified_installer_callback(NULL);
        Check(!ApplicationController::VerifiedInstallerCallback(file.c_str(), digest.data()), "missing callback fails closed");
        Reject([] { ApplicationController::RequireHardenedConfiguration(); });

        std::cerr << "native: download sink\n";
        FixtureThread thread;
        thread.Start(); thread.Join();
        wchar_t current[MAX_PATH];
        Check(GetCurrentDirectoryW(MAX_PATH, current) > 0, "fixture directory");
        {
            UpdateDownloadSink sink(thread, current, delivered.downloadToken);
            sink.SetFilename(L"..\\..\\outside.msi:stream");
            Check(sink.GetFilePath() == std::wstring(current) + L"\\payload.exe", "fixed NSIS pathname");
            sink.Add(payload.data(), payload.size()); sink.Close();
        }
        Check(DeleteFileW(L"payload.exe") != 0, "remove inert sink fixture");
        Check(DeleteFileW(file.c_str()) != 0, "remove signature fixture");
        std::cerr << "native: actual byte limits and XML/verifier bounds\n";
        TestByteBounds(current, sig);

        // Initialize wx only for hidden native text controls. Do not start the
        // SDK workers, call cleanup, or claim that cleanup permits DLL unload.
        std::cerr << "native: hidden notes control\n";
        wxSetAssertHandler([](const wxString& file, int line, const wxString&, const wxString& condition, const wxString& message) {
            std::cerr << "wx assertion: " << file.ToUTF8().data() << ':' << line << ' ' << condition.ToUTF8().data() << ' ' << message.ToUTF8().data() << '\n';
            throw std::runtime_error("wx assertion");
        });
        UI::SetDllHINSTANCE(GetModuleHandle(NULL));
        wxInitializer wx;
        Check(wx.IsOk(), "wx initialization");
        Check(wxTheApp->CallOnInit(), "wx application initialization");
        std::cerr << "native: wx initialized\n";
        {
            UpdateDialog dialog;
            std::cerr << "native: dialog constructed\n";
            Appcast info;
            info.Version = "2.0.0";
            info.Description = "<script>alert(1)</script><a href='file:///C:/'>literal</a>";
            info.ReleaseNotesURL = "https://must-not-be-loaded.invalid/notes.html";
            Settings::SetAppName(L"SDK gate fixture");
            Settings::SetAppVersion(L"1.0.0");
            std::cerr << "native: render notes\n";
            dialog.StateUpdateAvailable(info, true); // still requires confirmation
            wxTextCtrl* notes = nullptr;
            for (auto child : dialog.GetChildren())
                if (auto text = wxDynamicCast(child, wxTextCtrl)) notes = text;
            Check(notes && notes->GetValue() == wxString::FromUTF8(info.Description), "notes are literal text");
            Check(!notes->IsEditable(), "notes are read only");
            const int before = callbackCount;
            dialog.StateUpdateDownloaded(file, digest, delivered.downloadToken);
            Check(callbackCount == before, "completion without an active download is ignored");
        }
        std::cerr << "native: active offer and tagged worker events\n";
        win_sparkle_set_verified_installer_callback(Callback);
        TestActiveOfferIdentity(file, digest);
        SecureZeroMemory(seed.data(), seed.size());
        SecureZeroMemory(secret.data(), secret.size());
        SecureZeroMemory(otherSecret.data(), otherSecret.size());
        std::cout << "PASS: synchronous request ownership, loopback TLS error/timeout/cancel, periodic guard, byte bounds, callback locks, active offer identity, Ed25519/CNG identity, rejection cases, event copy, fixed path, inert notes\n";
        return 0;
    }
    catch (const std::exception& e)
    {
        std::cerr << "FAIL: " << e.what() << '\n';
        return 1;
    }
}
