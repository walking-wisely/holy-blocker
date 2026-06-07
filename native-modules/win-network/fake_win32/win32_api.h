// Drop-in replacement for <windows.h> and <wintun.h> used by the fake_win32
// shim.  When HOLY_BLOCKER_FAKE_WIN32 is defined, production source files
// include this header instead of the real SDK headers.

#pragma once

#include <cstdint>
#include <cstring>
#include <string>

// ──────────────────────────────────────────────────────────────────────────────
// Basic Windows type aliases
// ──────────────────────────────────────────────────────────────────────────────
using BOOL     = int;
using DWORD    = uint32_t;
using DWORD64  = uint64_t;
using HANDLE   = void*;
using HKEY     = void*;
using LONG     = int32_t;
using ULONG    = uint32_t;
using ULONG64  = uint64_t;
using WCHAR    = wchar_t;
using LPWSTR   = wchar_t*;
using LPCWSTR  = const wchar_t*;
using LPVOID   = void*;
using BYTE     = uint8_t;
using WORD     = uint16_t;
using SC_HANDLE = void*;

#define TRUE  1
#define FALSE 0
#define NO_ERROR 0UL
#define WINAPI
#define ERROR_ALREADY_EXISTS 183UL
#define ERROR_NOT_FOUND      1168UL

// ──────────────────────────────────────────────────────────────────────────────
// GUID
// ──────────────────────────────────────────────────────────────────────────────
struct GUID {
    uint32_t Data1;
    uint16_t Data2;
    uint16_t Data3;
    uint8_t  Data4[8];

    bool operator==(const GUID& o) const noexcept {
        return std::memcmp(this, &o, sizeof(GUID)) == 0;
    }
};

// ──────────────────────────────────────────────────────────────────────────────
// NET_LUID
// ──────────────────────────────────────────────────────────────────────────────
union NET_LUID {
    uint64_t Value;
    struct {
        uint64_t Reserved  : 24;
        uint64_t NetLuidIndex : 24;
        uint64_t IfType    : 16;
    } Info;
};

// ──────────────────────────────────────────────────────────────────────────────
// IP Helper / routing types (simplified stubs)
// ──────────────────────────────────────────────────────────────────────────────
struct SOCKADDR_INET {
    uint16_t si_family;
    uint8_t  _pad[14];
};

struct IP_ADDRESS_PREFIX {
    SOCKADDR_INET Prefix;
    uint8_t       PrefixLength;
    uint8_t       _pad[3];
};

struct MIB_IPFORWARD_ROW2 {
    NET_LUID          InterfaceLuid;
    ULONG             InterfaceIndex;
    IP_ADDRESS_PREFIX DestinationPrefix;
    SOCKADDR_INET     NextHop;
    BYTE              SitePrefixLength;
    ULONG             ValidLifetime;
    ULONG             PreferredLifetime;
    ULONG             Metric;
    ULONG             Protocol;
    BOOL              Loopback;
    BOOL              AutoconfigureAddress;
    BOOL              Publish;
    BOOL              Immortal;
    ULONG             Age;
    ULONG             Origin;
};

// ──────────────────────────────────────────────────────────────────────────────
// DNS settings stub
// ──────────────────────────────────────────────────────────────────────────────
#define DNS_INTERFACE_SETTINGS_VERSION1 1

struct DNS_INTERFACE_SETTINGS {
    ULONG    Version;
    ULONG64  Flags;
    LPWSTR   Domain;
    LPWSTR   NameServer;
    LPWSTR   SearchList;
    ULONG    RegistrationEnabled;
    ULONG    RegisterAdapterName;
    ULONG    EnableLLMNR;
    ULONG    QueryAdapterName;
    LPWSTR   ProfileNameServer;
};

// ──────────────────────────────────────────────────────────────────────────────
// Registry stubs
// ──────────────────────────────────────────────────────────────────────────────
#define HKEY_LOCAL_MACHINE reinterpret_cast<HKEY>(0x80000002LL)
#define REG_SZ    1UL
#define REG_BINARY 3UL
#define KEY_SET_VALUE   0x0002
#define KEY_QUERY_VALUE 0x0001
#define KEY_ALL_ACCESS  0xF003F

// ──────────────────────────────────────────────────────────────────────────────
// SCM stubs
// ──────────────────────────────────────────────────────────────────────────────
#define SERVICE_AUTO_START    2UL
#define SERVICE_ERROR_NORMAL  1UL
#define SERVICE_WIN32_OWN_PROCESS 0x10UL
#define SC_MANAGER_ALL_ACCESS 0xF003FUL
#define SERVICE_ALL_ACCESS    0xF01FFUL

// ──────────────────────────────────────────────────────────────────────────────
// Wintun types (re-declared here so wintun.h is not needed under the shim)
// ──────────────────────────────────────────────────────────────────────────────
using WINTUN_ADAPTER_HANDLE = void*;
using WINTUN_SESSION_HANDLE = void*;

// ──────────────────────────────────────────────────────────────────────────────
// Fake Win32 API declarations (implemented in fake_*.cpp)
// ──────────────────────────────────────────────────────────────────────────────

// IP Helper
DWORD InitializeIpForwardEntry(MIB_IPFORWARD_ROW2* row);
DWORD CreateIpForwardEntry2(const MIB_IPFORWARD_ROW2* row);
DWORD DeleteIpForwardEntry2(const MIB_IPFORWARD_ROW2* row);
DWORD SetInterfaceDnsSettings(GUID adapter_guid,
                               const DNS_INTERFACE_SETTINGS* settings);

// Registry
LONG RegCreateKeyExW(HKEY hKey, LPCWSTR lpSubKey, DWORD Reserved,
                     LPWSTR lpClass, DWORD dwOptions, DWORD samDesired,
                     void* lpSecurityAttributes, HKEY* phkResult,
                     DWORD* lpdwDisposition);
LONG RegSetValueExW(HKEY hKey, LPCWSTR lpValueName, DWORD Reserved,
                    DWORD dwType, const BYTE* lpData, DWORD cbData);
LONG RegGetValueW(HKEY hKey, LPCWSTR lpSubKey, LPCWSTR lpValue,
                  DWORD dwFlags, DWORD* pdwType, void* pvData,
                  DWORD* pcbData);
LONG RegDeleteKeyExW(HKEY hKey, LPCWSTR lpSubKey, DWORD samDesired,
                     DWORD Reserved);
LONG RegCloseKey(HKEY hKey);

// SCM
SC_HANDLE OpenSCManagerW(LPCWSTR lpMachineName, LPCWSTR lpDatabaseName,
                          DWORD dwDesiredAccess);
SC_HANDLE CreateServiceW(SC_HANDLE hSCManager, LPCWSTR lpServiceName,
                          LPCWSTR lpDisplayName, DWORD dwDesiredAccess,
                          DWORD dwServiceType, DWORD dwStartType,
                          DWORD dwErrorControl, LPCWSTR lpBinaryPathName,
                          LPCWSTR lpLoadOrderGroup, DWORD* lpdwTagId,
                          LPCWSTR lpDependencies, LPCWSTR lpServiceStartName,
                          LPCWSTR lpPassword);
SC_HANDLE OpenServiceW(SC_HANDLE hSCManager, LPCWSTR lpServiceName,
                        DWORD dwDesiredAccess);
BOOL      DeleteService(SC_HANDLE hService);
BOOL      CloseServiceHandle(SC_HANDLE hSCObject);

// Wintun (fake)
WINTUN_ADAPTER_HANDLE FakeWintunCreateAdapter(LPCWSTR Name,
                                               LPCWSTR TunnelType,
                                               const GUID* RequestedGUID);
WINTUN_ADAPTER_HANDLE FakeWintunOpenAdapter(LPCWSTR Name);
void                  FakeWintunCloseAdapter(WINTUN_ADAPTER_HANDLE Adapter);
BOOL                  FakeWintunDeleteAdapter(WINTUN_ADAPTER_HANDLE Adapter);
WINTUN_SESSION_HANDLE FakeWintunStartSession(WINTUN_ADAPTER_HANDLE Adapter,
                                              DWORD Capacity);
void                  FakeWintunEndSession(WINTUN_SESSION_HANDLE Session);
void                  FakeWintunGetAdapterLuid(WINTUN_ADAPTER_HANDLE Adapter,
                                                NET_LUID* Luid);

// Map real Wintun names to fake implementations when the shim is active
#define WintunCreateAdapter  FakeWintunCreateAdapter
#define WintunOpenAdapter    FakeWintunOpenAdapter
#define WintunCloseAdapter   FakeWintunCloseAdapter
#define WintunDeleteAdapter  FakeWintunDeleteAdapter
#define WintunStartSession   FakeWintunStartSession
#define WintunEndSession     FakeWintunEndSession
#define WintunGetAdapterLuid FakeWintunGetAdapterLuid
