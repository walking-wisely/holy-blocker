#include "win32_api.h"
#include "recorder.h"

// Fake handles — just distinguishable non-null pointers
static int g_scm_sentinel   = 1;
static int g_svc_sentinel   = 2;

SC_HANDLE OpenSCManagerW(LPCWSTR /*lpMachineName*/, LPCWSTR /*lpDatabaseName*/,
                          DWORD /*dwDesiredAccess*/) {
    return reinterpret_cast<SC_HANDLE>(&g_scm_sentinel);
}

SC_HANDLE CreateServiceW(SC_HANDLE /*hSCManager*/, LPCWSTR lpServiceName,
                          LPCWSTR lpDisplayName, DWORD /*dwDesiredAccess*/,
                          DWORD dwServiceType, DWORD dwStartType,
                          DWORD /*dwErrorControl*/, LPCWSTR lpBinaryPathName,
                          LPCWSTR /*lpLoadOrderGroup*/, DWORD* /*lpdwTagId*/,
                          LPCWSTR /*lpDependencies*/,
                          LPCWSTR /*lpServiceStartName*/,
                          LPCWSTR /*lpPassword*/) {
    FakeWin32::CreateServiceCall rec{};
    rec.service_name  = lpServiceName  ? lpServiceName  : L"";
    rec.display_name  = lpDisplayName  ? lpDisplayName  : L"";
    rec.binary_path   = lpBinaryPathName ? lpBinaryPathName : L"";
    rec.start_type    = dwStartType;
    rec.service_type  = dwServiceType;
    FakeWin32::CallLog::Get().CreateService.push_back(std::move(rec));
    return reinterpret_cast<SC_HANDLE>(&g_svc_sentinel);
}

SC_HANDLE OpenServiceW(SC_HANDLE /*hSCManager*/, LPCWSTR /*lpServiceName*/,
                        DWORD /*dwDesiredAccess*/) {
    return reinterpret_cast<SC_HANDLE>(&g_svc_sentinel);
}

BOOL DeleteService(SC_HANDLE /*hService*/) {
    return TRUE;
}

BOOL CloseServiceHandle(SC_HANDLE /*hSCObject*/) {
    return TRUE;
}
