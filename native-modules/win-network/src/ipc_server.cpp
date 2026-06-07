#include "ipc_server.h"

#ifdef HOLY_BLOCKER_FAKE_WIN32
#include "win32_api.h"
#else
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <sddl.h>
#endif

#include <string>
#include <string_view>
#include <array>

// ──────────────────────────────────────────────────────────────────────────────
// Minimal flat-JSON helpers (no external library)
// ──────────────────────────────────────────────────────────────────────────────
namespace {

// Extracts the first unescaped string value for `key` in a flat JSON object.
// Returns empty string if the key is absent or the value is not a string.
std::string extract_string(std::string_view json, std::string_view key) {
    std::string search;
    search.reserve(key.size() + 2);
    search += '"';
    search += key;
    search += '"';

    auto pos = json.find(search);
    if (pos == std::string_view::npos) return {};
    pos += search.size();

    while (pos < json.size() && (json[pos] == ' ' || json[pos] == ':')) ++pos;
    if (pos >= json.size() || json[pos] != '"') return {};
    ++pos;

    std::string result;
    while (pos < json.size() && json[pos] != '"') {
        if (json[pos] == '\\' && pos + 1 < json.size()) {
            ++pos;  // consume backslash, copy next char verbatim
        }
        result += json[pos++];
    }
    return result;
}

std::string error_response(std::string_view m) {
    std::string r = R"({"ok":false,"error":")";
    r += m;
    r += "\"}";
    return r;
}

// ──────────────────────────────────────────────────────────────────────────────
// Named pipe constants
// ──────────────────────────────────────────────────────────────────────────────
constexpr LPCWSTR kPipeName    = L"\\\\.\\pipe\\HolyBlockerNetSvc";
constexpr DWORD   kBufferBytes = 4096;
// DACL: allow read/write to Administrators (BA) and SYSTEM (SY), deny all others.
constexpr LPCWSTR kPipeSddl =
    L"D:(A;;GRGW;;;BA)(A;;GRGW;;;SY)";

} // namespace

// ──────────────────────────────────────────────────────────────────────────────
// IpcServer
// ──────────────────────────────────────────────────────────────────────────────
IpcServer::IpcServer(CommandHandler& handler) : handler_(handler) {
    stop_event_ = CreateEventW(/*lpEventAttributes=*/nullptr,
                               /*bManualReset=*/TRUE,
                               /*bInitialState=*/FALSE,
                               /*lpName=*/nullptr);
}

IpcServer::~IpcServer() {
    if (stop_event_) CloseHandle(static_cast<HANDLE>(stop_event_));
}

void IpcServer::Stop() {
    if (stop_event_) SetEvent(static_cast<HANDLE>(stop_event_));
}

std::string IpcServer::Dispatch(const std::string& json_command) {
    std::string cmd = extract_string(json_command, "cmd");
    if (cmd.empty())           return error_response("missing cmd field");
    if (cmd == "start")        return handler_.OnStart();
    if (cmd == "stop")         return handler_.OnStop();
    if (cmd == "status")       return handler_.OnStatus();
    if (cmd == "reload_rules") {
        std::string path = extract_string(json_command, "path");
        if (path.empty()) return error_response("missing path field");
        return handler_.OnReloadRules(path);
    }
    return error_response("unknown command: " + cmd);
}

// ──────────────────────────────────────────────────────────────────────────────
// Run — named pipe server loop
//
// One connection at a time, overlapped ConnectNamedPipe so Stop() can interrupt
// the wait.  Each connection is a single newline-terminated JSON request followed
// by a single newline-terminated JSON response; then the connection closes.
// ──────────────────────────────────────────────────────────────────────────────
void IpcServer::Run() {
    if (!stop_event_) return;

    HANDLE stop_ev = static_cast<HANDLE>(stop_event_);

    // Build security descriptor from SDDL so only Administrators / SYSTEM can
    // connect.  The handle is owned by this scope; free with LocalFree.
    PSECURITY_DESCRIPTOR sd = nullptr;
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
            kPipeSddl, SDDL_REVISION_1, &sd, /*lpdwSDSize=*/nullptr)) {
        return;
    }

    SECURITY_ATTRIBUTES sa{};
    sa.nLength              = sizeof(sa);
    sa.lpSecurityDescriptor = sd;
    sa.bInheritHandle       = FALSE;

    // Overlapped event for ConnectNamedPipe.
    HANDLE connect_ev = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!connect_ev) {
        LocalFree(sd);
        return;
    }

    while (true) {
        ResetEvent(connect_ev);

        HANDLE pipe = CreateNamedPipeW(
            kPipeName,
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            /*nMaxInstances=*/1,
            kBufferBytes,
            kBufferBytes,
            /*nDefaultTimeOut=*/0,
            &sa);

        if (pipe == INVALID_HANDLE_VALUE) break;

        // Wait for a client to connect or for the stop signal.
        OVERLAPPED ov{};
        ov.hEvent = connect_ev;
        BOOL connected = ConnectNamedPipe(pipe, &ov);
        if (!connected) {
            DWORD err = GetLastError();
            if (err == ERROR_IO_PENDING) {
                HANDLE events[2] = {stop_ev, connect_ev};
                DWORD  wait = WaitForMultipleObjects(2, events, FALSE, INFINITE);
                if (wait != WAIT_OBJECT_0 + 1) {
                    // Stop signal or error — clean up and exit.
                    CloseHandle(pipe);
                    break;
                }
                DWORD ignored{};
                connected = GetOverlappedResult(pipe, &ov, &ignored, FALSE);
            } else if (err == ERROR_PIPE_CONNECTED) {
                connected = TRUE;
            }
        }

        if (connected) {
            // Read request (newline-terminated, up to kBufferBytes).
            std::string request;
            request.resize(kBufferBytes);
            DWORD bytes_read = 0;
            OVERLAPPED read_ov{};
            read_ov.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
            BOOL ok = ReadFile(pipe, request.data(),
                               static_cast<DWORD>(request.size()),
                               &bytes_read, &read_ov);
            if (!ok && GetLastError() == ERROR_IO_PENDING) {
                HANDLE events[2] = {stop_ev, read_ov.hEvent};
                if (WaitForMultipleObjects(2, events, FALSE, INFINITE) == WAIT_OBJECT_0 + 1)
                    ok = GetOverlappedResult(pipe, &read_ov, &bytes_read, FALSE);
            }
            CloseHandle(read_ov.hEvent);

            if (ok && bytes_read > 0) {
                request.resize(bytes_read);
                // Strip trailing newline/whitespace.
                while (!request.empty() && (request.back() == '\n' || request.back() == '\r'))
                    request.pop_back();

                std::string response = Dispatch(request);
                response += '\n';

                DWORD bytes_written = 0;
                WriteFile(pipe, response.data(),
                          static_cast<DWORD>(response.size()),
                          &bytes_written, nullptr);
            }
        }

        DisconnectNamedPipe(pipe);
        CloseHandle(pipe);

        // Check stop signal before looping.
        if (WaitForSingleObject(stop_ev, 0) == WAIT_OBJECT_0) break;
    }

    CloseHandle(connect_ev);
    LocalFree(sd);
}
