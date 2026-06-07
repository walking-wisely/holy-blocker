#include "win32_api.h"
#include "recorder.h"

DWORD InitializeIpForwardEntry(MIB_IPFORWARD_ROW2* row) {
    if (!row) return ERROR_NOT_FOUND;
    *row = {};
    return NO_ERROR;
}

DWORD CreateIpForwardEntry2(const MIB_IPFORWARD_ROW2* row) {
    auto& log = FakeWin32::CallLog::Get();
    DWORD result = log.next_CreateIpForwardEntry2_result;
    log.CreateIpForwardEntry2.push_back({*row, result});
    return result;
}

DWORD DeleteIpForwardEntry2(const MIB_IPFORWARD_ROW2* row) {
    auto& log = FakeWin32::CallLog::Get();
    DWORD result = log.next_DeleteIpForwardEntry2_result;
    log.DeleteIpForwardEntry2.push_back({*row});
    return result;
}

DWORD SetInterfaceDnsSettings(GUID adapter_guid,
                               const DNS_INTERFACE_SETTINGS* settings) {
    auto& log = FakeWin32::CallLog::Get();
    DWORD result = log.next_SetInterfaceDnsSettings_result;
    FakeWin32::SetInterfaceDnsSettingsCall rec{};
    rec.adapter_guid = adapter_guid;
    rec.settings     = *settings;
    if (settings->NameServer) {
        rec.name_server = settings->NameServer;
    }
    log.SetInterfaceDnsSettings.push_back(std::move(rec));
    return result;
}

DWORD ConvertInterfaceLuidToGuid(const NET_LUID* luid, GUID* guid) {
    auto& log = FakeWin32::CallLog::Get();
    DWORD result = log.next_ConvertInterfaceLuidToGuid_result;
    FakeWin32::ConvertInterfaceLuidToGuidCall rec{};
    rec.luid = *luid;
    if (result == NO_ERROR) {
        *guid    = log.next_ConvertInterfaceLuidToGuid_guid;
        rec.out_guid = *guid;
    }
    log.ConvertInterfaceLuidToGuid.push_back(rec);
    return result;
}
