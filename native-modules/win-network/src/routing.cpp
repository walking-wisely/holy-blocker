#include "routing.h"

static std::error_code win_error(DWORD code) {
    return std::error_code(static_cast<int>(code), std::system_category());
}

// Metric added on top of the current default gateway metric so our route is
// preferred without removing internet access on the real adapter.
static constexpr ULONG kRouteMetric = 1;

std::expected<void, std::error_code>
RoutingManager::AddDefaultRoute(NET_LUID adapter_luid) {
    if (route_installed_) return {};  // idempotent

    MIB_IPFORWARD_ROW2 row{};
    InitializeIpForwardEntry(&row);

    row.InterfaceLuid                        = adapter_luid;
    row.DestinationPrefix.PrefixLength       = 0;   // 0.0.0.0/0 — default route
    row.DestinationPrefix.Prefix.si_family   = 2;   // AF_INET
    row.NextHop.si_family                    = 2;   // AF_INET (gateway 0.0.0.0)
    row.Metric                               = kRouteMetric;
    row.Protocol                             = 3;   // MIB_IPPROTO_NETMGMT

    DWORD err = CreateIpForwardEntry2(&row);
    if (err != NO_ERROR) return std::unexpected(win_error(err));

    route_           = row;
    route_installed_ = true;
    return {};
}

std::expected<void, std::error_code>
RoutingManager::RemoveDefaultRoute(NET_LUID adapter_luid) {
    if (!route_installed_) return {};  // idempotent

    DWORD err = DeleteIpForwardEntry2(&route_);
    if (err != NO_ERROR) return std::unexpected(win_error(err));

    route_installed_ = false;
    return {};
}

std::expected<void, std::error_code>
RoutingManager::SetDnsServers(NET_LUID adapter_luid,
                               const std::vector<std::wstring>& servers) {
    GUID guid{};
    DWORD err = ConvertInterfaceLuidToGuid(&adapter_luid, &guid);
    if (err != NO_ERROR) return std::unexpected(win_error(err));

    // Build semicolon-delimited nameserver string
    std::wstring ns_list;
    for (std::size_t i = 0; i < servers.size(); ++i) {
        if (i) ns_list += L';';
        ns_list += servers[i];
    }

    DNS_INTERFACE_SETTINGS settings{};
    settings.Version    = DNS_INTERFACE_SETTINGS_VERSION1;
    settings.Flags      = DNS_SETTING_NAMESERVER;
    settings.NameServer = ns_list.empty() ? nullptr : ns_list.data();

    err = SetInterfaceDnsSettings(guid, &settings);
    if (err != NO_ERROR) return std::unexpected(win_error(err));
    return {};
}

std::expected<void, std::error_code>
RoutingManager::ClearDnsServers(NET_LUID adapter_luid) {
    GUID guid{};
    DWORD err = ConvertInterfaceLuidToGuid(&adapter_luid, &guid);
    if (err != NO_ERROR) return std::unexpected(win_error(err));

    DNS_INTERFACE_SETTINGS settings{};
    settings.Version    = DNS_INTERFACE_SETTINGS_VERSION1;
    settings.Flags      = DNS_SETTING_NAMESERVER;
    settings.NameServer = nullptr;

    err = SetInterfaceDnsSettings(guid, &settings);
    if (err != NO_ERROR) return std::unexpected(win_error(err));
    return {};
}
