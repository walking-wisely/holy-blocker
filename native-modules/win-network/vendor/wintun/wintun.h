// Wintun public API header.
// The real wintun.h is downloaded from https://www.wintun.net/ and placed here.
// Only the header is checked in; the DLL is shipped with the installer.
//
// This stub declares the types and function pointer typedefs used by
// wintun_adapter.cpp.  Replace with the real wintun.h before building the
// production executable.

#pragma once

#ifndef HOLY_BLOCKER_FAKE_WIN32
#  include <windows.h>
#endif

// Opaque handle types
typedef void* WINTUN_ADAPTER_HANDLE;
typedef void* WINTUN_SESSION_HANDLE;

typedef enum {
    WINTUN_LOG_INFO,
    WINTUN_LOG_WARN,
    WINTUN_LOG_ERR,
} WINTUN_LOGGER_LEVEL;

typedef void(WINAPI* WINTUN_LOGGER_CALLBACK)(WINTUN_LOGGER_LEVEL Level,
                                              DWORD64 Timestamp,
                                              const WCHAR* Message);

// Function pointer types that match the real Wintun ABI.
typedef WINTUN_ADAPTER_HANDLE(WINAPI* WINTUN_CREATE_ADAPTER_FUNC)(
    const WCHAR* Name, const WCHAR* TunnelType, const GUID* RequestedGUID);

typedef WINTUN_ADAPTER_HANDLE(WINAPI* WINTUN_OPEN_ADAPTER_FUNC)(
    const WCHAR* Name);

typedef void(WINAPI* WINTUN_CLOSE_ADAPTER_FUNC)(WINTUN_ADAPTER_HANDLE Adapter);

typedef BOOL(WINAPI* WINTUN_DELETE_ADAPTER_FUNC)(WINTUN_ADAPTER_HANDLE Adapter);

typedef void(WINAPI* WINTUN_GET_ADAPTER_LUID_FUNC)(
    WINTUN_ADAPTER_HANDLE Adapter, NET_LUID* Luid);

typedef WINTUN_SESSION_HANDLE(WINAPI* WINTUN_START_SESSION_FUNC)(
    WINTUN_ADAPTER_HANDLE Adapter, DWORD Capacity);

typedef void(WINAPI* WINTUN_END_SESSION_FUNC)(WINTUN_SESSION_HANDLE Session);

typedef void(WINAPI* WINTUN_SET_LOGGER_FUNC)(WINTUN_LOGGER_CALLBACK NewLogger);

#define WINTUN_MIN_RING_CAPACITY 0x20000    /* 128 KiB */
#define WINTUN_MAX_RING_CAPACITY 0x4000000  /* 64 MiB */
