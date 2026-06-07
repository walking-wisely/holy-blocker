#pragma once

#include "win32_api.h"
#include <vector>
#include <string>

namespace FakeWin32 {

// ──────────────────────────────────────────────────────────────────────────────
// Per-API call records
// ──────────────────────────────────────────────────────────────────────────────

struct CreateIpForwardEntry2Call {
    MIB_IPFORWARD_ROW2 row;
    DWORD              returned;
};

struct DeleteIpForwardEntry2Call {
    MIB_IPFORWARD_ROW2 row;
};

struct SetInterfaceDnsSettingsCall {
    GUID                  adapter_guid;
    DNS_INTERFACE_SETTINGS settings;
    std::wstring          name_server;  // copy of settings.NameServer
};

struct RegSetValueCall {
    std::wstring key_path;
    std::wstring value_name;
    DWORD        type;
    std::vector<uint8_t> data;
};

struct RegGetValueCall {
    std::wstring key_path;
    std::wstring value_name;
};

struct CreateServiceCall {
    std::wstring service_name;
    std::wstring display_name;
    std::wstring binary_path;
    DWORD        start_type;
    DWORD        service_type;
};

struct WintunCreateAdapterCall {
    std::wstring name;
    std::wstring tunnel_type;
    GUID         requested_guid;
    WINTUN_ADAPTER_HANDLE returned_handle;
};

struct WintunOpenAdapterCall {
    std::wstring name;
    WINTUN_ADAPTER_HANDLE returned_handle;
};

struct WintunDeleteAdapterCall {
    WINTUN_ADAPTER_HANDLE handle;
};

// ──────────────────────────────────────────────────────────────────────────────
// CallLog singleton — tests inspect and reset between cases
// ──────────────────────────────────────────────────────────────────────────────
struct CallLog {
    std::vector<CreateIpForwardEntry2Call>    CreateIpForwardEntry2;
    std::vector<DeleteIpForwardEntry2Call>    DeleteIpForwardEntry2;
    std::vector<SetInterfaceDnsSettingsCall>  SetInterfaceDnsSettings;
    std::vector<RegSetValueCall>              RegSetValue;
    std::vector<RegGetValueCall>              RegGetValue;
    std::vector<CreateServiceCall>            CreateService;
    std::vector<WintunCreateAdapterCall>      WintunCreateAdapter;
    std::vector<WintunOpenAdapterCall>        WintunOpenAdapter;
    std::vector<WintunDeleteAdapterCall>      WintunDeleteAdapter;

    // Configurable return values for next call
    DWORD next_CreateIpForwardEntry2_result = NO_ERROR;
    DWORD next_DeleteIpForwardEntry2_result = NO_ERROR;
    DWORD next_SetInterfaceDnsSettings_result = NO_ERROR;

    static CallLog& Get();
    static void     Reset();
};

    // Reset the in-memory registry (separate from CallLog so tests can clear
    // each independently).
    void ResetRegistry();

} // namespace FakeWin32
