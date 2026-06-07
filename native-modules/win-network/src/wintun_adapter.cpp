#include "wintun_adapter.h"

#include <optional>

static constexpr LPCWSTR kRegKey     = L"SOFTWARE\\HolyBlocker\\NetSvc";
static constexpr LPCWSTR kValueGuid  = L"AdapterGUID";
static constexpr LPCWSTR kValueName  = L"AdapterName";
static constexpr LPCWSTR kTunnelType = L"HolyBlocker";
static constexpr DWORD   kRingBytes  = 4u * 1024u * 1024u;

// Fixed GUID assigned to the HolyBlocker virtual adapter.  Using a fixed GUID
// lets the service reclaim the adapter across restarts without extra bookkeeping.
static constexpr GUID kAdapterGuid = {
    0xd344d8e4u, 0x3fdfu, 0x4d01u,
    {0xb5u, 0x9cu, 0x11u, 0x22u, 0x33u, 0x44u, 0x55u, 0x66u}
};

// ──────────────────────────────────────────────────────────────────────────────
// Wintun DLL function pointers
//
// Wintun does not ship an import library.  The API is loaded at runtime from
// wintun.dll via GetProcAddress.  The fake_win32 shim covers these names with
// #defines, so this block is compiled only in production mode.
// ──────────────────────────────────────────────────────────────────────────────
#ifndef HOLY_BLOCKER_FAKE_WIN32
static HMODULE                      g_wintun_dll         = nullptr;
static WINTUN_CREATE_ADAPTER_FUNC   WintunCreateAdapter  = nullptr;
static WINTUN_OPEN_ADAPTER_FUNC     WintunOpenAdapter    = nullptr;
static WINTUN_CLOSE_ADAPTER_FUNC    WintunCloseAdapter   = nullptr;
static WINTUN_DELETE_ADAPTER_FUNC   WintunDeleteAdapter  = nullptr;
static WINTUN_START_SESSION_FUNC    WintunStartSession   = nullptr;
static WINTUN_END_SESSION_FUNC      WintunEndSession     = nullptr;
static WINTUN_GET_ADAPTER_LUID_FUNC WintunGetAdapterLuid = nullptr;
#endif

// ──────────────────────────────────────────────────────────────────────────────
// Registry helpers
// ──────────────────────────────────────────────────────────────────────────────

static bool SaveToRegistry(const GUID& guid, std::wstring_view name) {
    HKEY hk{};
    LONG r = RegCreateKeyExW(HKEY_LOCAL_MACHINE, kRegKey,
                             0, nullptr, 0, KEY_SET_VALUE,
                             nullptr, &hk, nullptr);
    if (r != 0) return false;

    r = RegSetValueExW(hk, kValueGuid, 0, REG_BINARY,
                       reinterpret_cast<const BYTE*>(&guid),
                       static_cast<DWORD>(sizeof(GUID)));
    if (r != 0) { RegCloseKey(hk); return false; }

    r = RegSetValueExW(hk, kValueName, 0, REG_BINARY,
                       reinterpret_cast<const BYTE*>(name.data()),
                       static_cast<DWORD>(name.size() * sizeof(wchar_t)));
    RegCloseKey(hk);
    return r == 0;
}

static std::optional<std::wstring> ReadAdapterName() {
    DWORD size = 0;
    LONG r = RegGetValueW(HKEY_LOCAL_MACHINE, kRegKey, kValueName,
                          0, nullptr, nullptr, &size);
    if (r != 0 || size < sizeof(wchar_t)) return std::nullopt;

    std::wstring name(size / sizeof(wchar_t), L'\0');
    r = RegGetValueW(HKEY_LOCAL_MACHINE, kRegKey, kValueName,
                     0, nullptr, name.data(), &size);
    if (r != 0) return std::nullopt;
    return name;
}

static std::error_code win_error(DWORD code) {
    return std::error_code(static_cast<int>(code), std::system_category());
}

// ──────────────────────────────────────────────────────────────────────────────
// WintunAdapter
// ──────────────────────────────────────────────────────────────────────────────

std::error_code WintunAdapter::LoadWintunDll() {
#ifndef HOLY_BLOCKER_FAKE_WIN32
    g_wintun_dll = LoadLibraryExW(L"wintun.dll", nullptr,
                                   LOAD_LIBRARY_SEARCH_SYSTEM32 |
                                   LOAD_LIBRARY_SEARCH_APPLICATION_DIR);
    if (!g_wintun_dll) return win_error(GetLastError());

    // Resolve all function pointers in one pass; any missing export is fatal.
#define GET(name) reinterpret_cast<decltype(name)>(GetProcAddress(g_wintun_dll, #name))
    WintunCreateAdapter  = GET(WintunCreateAdapter);
    WintunOpenAdapter    = GET(WintunOpenAdapter);
    WintunCloseAdapter   = GET(WintunCloseAdapter);
    WintunDeleteAdapter  = GET(WintunDeleteAdapter);
    WintunStartSession   = GET(WintunStartSession);
    WintunEndSession     = GET(WintunEndSession);
    WintunGetAdapterLuid = GET(WintunGetAdapterLuid);
#undef GET

    if (!WintunCreateAdapter || !WintunOpenAdapter || !WintunCloseAdapter ||
        !WintunDeleteAdapter || !WintunStartSession || !WintunEndSession  ||
        !WintunGetAdapterLuid)
        return win_error(ERROR_PROC_NOT_FOUND);
#endif
    return {};
}

std::expected<WintunAdapter, std::error_code>
WintunAdapter::Install(std::wstring_view name) {
    GUID guid = kAdapterGuid;
    WINTUN_ADAPTER_HANDLE h =
        WintunCreateAdapter(name.data(), kTunnelType, &guid);
    if (!h) {
#ifdef HOLY_BLOCKER_FAKE_WIN32
        return std::unexpected(win_error(ERROR_NOT_FOUND));
#else
        return std::unexpected(win_error(GetLastError()));
#endif
    }

    if (!SaveToRegistry(guid, name)) {
        WintunCloseAdapter(h);
        return std::unexpected(win_error(ERROR_NOT_FOUND));
    }

    return WintunAdapter(h, guid);
}

std::expected<WintunAdapter, std::error_code>
WintunAdapter::Open() {
    auto name_opt = ReadAdapterName();
    if (!name_opt) return Install(L"HolyBlocker");

    WINTUN_ADAPTER_HANDLE h = WintunOpenAdapter(name_opt->c_str());
    if (!h) return Install(*name_opt);

    GUID guid{};
    DWORD size = static_cast<DWORD>(sizeof(GUID));
    RegGetValueW(HKEY_LOCAL_MACHINE, kRegKey, kValueGuid,
                 0, nullptr, &guid, &size);

    return WintunAdapter(h, guid);
}

WINTUN_SESSION_HANDLE WintunAdapter::StartSession() {
    session_ = WintunStartSession(handle_, kRingBytes);
    return session_;
}

void WintunAdapter::Close() {
    if (session_) {
        WintunEndSession(session_);
        session_ = nullptr;
    }
    if (handle_) {
        WintunCloseAdapter(handle_);
        handle_ = nullptr;
    }
}

void WintunAdapter::Uninstall() {
    auto name_opt = ReadAdapterName();
    if (name_opt) {
        WINTUN_ADAPTER_HANDLE h = WintunOpenAdapter(name_opt->c_str());
        if (h) WintunDeleteAdapter(h);
    }
    RegDeleteKeyExW(HKEY_LOCAL_MACHINE, kRegKey, KEY_ALL_ACCESS, 0);
}

NET_LUID WintunAdapter::Luid() const {
    NET_LUID luid{};
    WintunGetAdapterLuid(handle_, &luid);
    return luid;
}
