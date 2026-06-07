#pragma once

#ifdef HOLY_BLOCKER_FAKE_WIN32
#  include "win32_api.h"
#else
#  include <windows.h>
#  include <netioapi.h>
#endif

#include <expected>
#include <string>
#include <system_error>
#include <vector>

class RoutingManager {
public:
    std::expected<void, std::error_code> AddDefaultRoute(NET_LUID adapter_luid);
    std::expected<void, std::error_code> RemoveDefaultRoute(NET_LUID adapter_luid);
    std::expected<void, std::error_code> SetDnsServers(NET_LUID adapter_luid,
                                                        const std::vector<std::wstring>& servers);
    std::expected<void, std::error_code> ClearDnsServers(NET_LUID adapter_luid);

private:
    // Stored to support idempotent removal
    MIB_IPFORWARD_ROW2 route_{};
    bool               route_installed_{false};
};
