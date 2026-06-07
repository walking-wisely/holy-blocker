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
using BOOLEAN  = uint8_t;   // distinct from BOOL; used in BOOLEAN fields of Win32 structs
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
#define ERROR_ALREADY_EXISTS  183UL
#define ERROR_NOT_FOUND      1168UL
#define ERROR_PROC_NOT_FOUND  127UL

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
    ULONG             InterfaceIndex;      // NET_IFINDEX
    IP_ADDRESS_PREFIX DestinationPrefix;
    SOCKADDR_INET     NextHop;
    BYTE              SitePrefixLength;    // UCHAR in real header
    ULONG             ValidLifetime;
    ULONG             PreferredLifetime;
    ULONG             Metric;
    ULONG             Protocol;            // NL_ROUTE_PROTOCOL enum
    BOOLEAN           Loopback;
    BOOLEAN           AutoconfigureAddress;
    BOOLEAN           Publish;
    BOOLEAN           Immortal;
    ULONG             Age;
    ULONG             Origin;              // NL_ROUTE_ORIGIN enum
};

// ──────────────────────────────────────────────────────────────────────────────
// DNS settings stub
// ──────────────────────────────────────────────────────────────────────────────
#define DNS_INTERFACE_SETTINGS_VERSION1 1
#define DNS_SETTING_IPV6                0x0001ULL
#define DNS_SETTING_NAMESERVER          0x0002ULL   // matches netioapi.h
#define DNS_SETTING_SEARCHLIST          0x0004ULL

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
// Named pipe / event / OVERLAPPED stubs
// ──────────────────────────────────────────────────────────────────────────────
#define INVALID_HANDLE_VALUE reinterpret_cast<HANDLE>(-1LL)

#define PIPE_ACCESS_DUPLEX              0x00000003UL
#define FILE_FLAG_OVERLAPPED            0x40000000UL
#define FILE_FLAG_FIRST_PIPE_INSTANCE   0x00080000UL
#define PIPE_TYPE_BYTE                  0x00000000UL
#define PIPE_READMODE_BYTE              0x00000000UL
#define PIPE_WAIT                       0x00000000UL
#define GENERIC_READ                    0x80000000UL
#define GENERIC_WRITE                   0x40000000UL

#define ERROR_IO_PENDING        997UL
#define ERROR_PIPE_CONNECTED    535UL
#define ERROR_BROKEN_PIPE       109UL

#define WAIT_OBJECT_0   0x00000000UL
#define WAIT_TIMEOUT    0x00000102UL
#define WAIT_ABANDONED  0x00000080UL
#define INFINITE        0xFFFFFFFFUL

#define SDDL_REVISION_1 1

struct OVERLAPPED {
    uintptr_t Internal;
    uintptr_t InternalHigh;
    DWORD     Offset;
    DWORD     OffsetHigh;
    HANDLE    hEvent;
};

using LPOVERLAPPED = OVERLAPPED*;
using PSECURITY_DESCRIPTOR = void*;

struct SECURITY_ATTRIBUTES {
    DWORD  nLength;
    LPVOID lpSecurityDescriptor;
    BOOL   bInheritHandle;
};

using LPSECURITY_ATTRIBUTES = SECURITY_ATTRIBUTES*;

// ──────────────────────────────────────────────────────────────────────────────
// Wintun types (re-declared here so wintun.h is not needed under the shim)
// ──────────────────────────────────────────────────────────────────────────────
using WINTUN_ADAPTER_HANDLE = void*;
using WINTUN_SESSION_HANDLE = void*;

// ──────────────────────────────────────────────────────────────────────────────
// Fake Win32 API declarations (implemented in fake_*.cpp)
// ──────────────────────────────────────────────────────────────────────────────

// IP Helper
void  InitializeIpForwardEntry(MIB_IPFORWARD_ROW2* row);  // real API returns VOID
DWORD CreateIpForwardEntry2(const MIB_IPFORWARD_ROW2* row);
DWORD DeleteIpForwardEntry2(const MIB_IPFORWARD_ROW2* row);
DWORD SetInterfaceDnsSettings(GUID adapter_guid,
                               const DNS_INTERFACE_SETTINGS* settings);
DWORD ConvertInterfaceLuidToGuid(const NET_LUID* luid, GUID* guid);

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

// Named pipe / event / wait (implemented in fake_pipe.cpp)
HANDLE CreateNamedPipeW(LPCWSTR lpName, DWORD dwOpenMode, DWORD dwPipeMode,
                        DWORD nMaxInstances, DWORD nOutBufferSize,
                        DWORD nInBufferSize, DWORD nDefaultTimeOut,
                        LPSECURITY_ATTRIBUTES lpSecurityAttributes);
BOOL   ConnectNamedPipe(HANDLE hNamedPipe, LPOVERLAPPED lpOverlapped);
BOOL   DisconnectNamedPipe(HANDLE hNamedPipe);
BOOL   ReadFile(HANDLE hFile, void* lpBuffer, DWORD nNumberOfBytesToRead,
                DWORD* lpNumberOfBytesRead, LPOVERLAPPED lpOverlapped);
BOOL   WriteFile(HANDLE hFile, const void* lpBuffer, DWORD nNumberOfBytesToWrite,
                 DWORD* lpNumberOfBytesWritten, LPOVERLAPPED lpOverlapped);
BOOL   CloseHandle(HANDLE hObject);
HANDLE CreateEventW(LPSECURITY_ATTRIBUTES lpEventAttributes, BOOL bManualReset,
                    BOOL bInitialState, LPCWSTR lpName);
BOOL   SetEvent(HANDLE hEvent);
BOOL   ResetEvent(HANDLE hEvent);
DWORD  WaitForSingleObject(HANDLE hHandle, DWORD dwMilliseconds);
DWORD  WaitForMultipleObjects(DWORD nCount, const HANDLE* lpHandles,
                               BOOL bWaitAll, DWORD dwMilliseconds);
BOOL   GetOverlappedResult(HANDLE hFile, LPOVERLAPPED lpOverlapped,
                            DWORD* lpNumberOfBytesTransferred, BOOL bWait);
DWORD  GetLastError();
BOOL   ConvertStringSecurityDescriptorToSecurityDescriptorW(
           LPCWSTR StringSecurityDescriptor, DWORD StringSDRevision,
           PSECURITY_DESCRIPTOR* SecurityDescriptor, DWORD* SecurityDescriptorSize);
void*  LocalFree(void* hMem);
