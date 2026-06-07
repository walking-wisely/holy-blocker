#include "win32_api.h"
#include "recorder.h"
#include <cstdlib>

// Fake adapter/session storage: opaque non-null sentinels per call
// (tests don't dereference these — they inspect the CallLog instead)

WINTUN_ADAPTER_HANDLE FakeWintunCreateAdapter(LPCWSTR Name,
                                               LPCWSTR TunnelType,
                                               const GUID* RequestedGUID) {
    auto& log = FakeWin32::CallLog::Get();
    // Allocate a unique sentinel so handles are distinguishable
    auto* sentinel = new int(static_cast<int>(log.WintunCreateAdapter.size()));
    WINTUN_ADAPTER_HANDLE h = sentinel;

    FakeWin32::WintunCreateAdapterCall rec{};
    rec.name             = Name ? Name : L"";
    rec.tunnel_type      = TunnelType ? TunnelType : L"";
    rec.requested_guid   = RequestedGUID ? *RequestedGUID : GUID{};
    rec.returned_handle  = h;
    log.WintunCreateAdapter.push_back(std::move(rec));
    return h;
}

WINTUN_ADAPTER_HANDLE FakeWintunOpenAdapter(LPCWSTR Name) {
    auto& log = FakeWin32::CallLog::Get();
    auto* sentinel = new int(static_cast<int>(log.WintunOpenAdapter.size()));
    WINTUN_ADAPTER_HANDLE h = sentinel;

    FakeWin32::WintunOpenAdapterCall rec{};
    rec.name            = Name ? Name : L"";
    rec.returned_handle = h;
    log.WintunOpenAdapter.push_back(std::move(rec));
    return h;
}

void FakeWintunCloseAdapter(WINTUN_ADAPTER_HANDLE Adapter) {
    delete reinterpret_cast<int*>(Adapter);
}

BOOL FakeWintunDeleteAdapter(WINTUN_ADAPTER_HANDLE Adapter) {
    FakeWin32::CallLog::Get().WintunDeleteAdapter.push_back({Adapter});
    delete reinterpret_cast<int*>(Adapter);
    return TRUE;
}

WINTUN_SESSION_HANDLE FakeWintunStartSession(WINTUN_ADAPTER_HANDLE /*Adapter*/,
                                              DWORD /*Capacity*/) {
    static int session_sentinel = 0;
    return &session_sentinel;
}

void FakeWintunEndSession(WINTUN_SESSION_HANDLE /*Session*/) {}

void FakeWintunGetAdapterLuid(WINTUN_ADAPTER_HANDLE Adapter, NET_LUID* Luid) {
    if (!Luid) return;
    // Derive a stable fake LUID from the adapter pointer value
    Luid->Value = reinterpret_cast<uint64_t>(Adapter);
}
