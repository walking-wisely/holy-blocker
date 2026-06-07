// Layer 1 tests — fake_win32 shim, no elevation needed.
// Covers RoutingManager against recorded IP Helper API calls.

#include <gtest/gtest.h>
#include "recorder.h"
#include "../src/routing.h"

namespace {

NET_LUID make_luid(uint64_t value) {
    NET_LUID luid{};
    luid.Value = value;
    return luid;
}

GUID make_guid(uint32_t d1) {
    GUID g{};
    g.Data1 = d1;
    return g;
}

} // namespace

// ─────────────────────────────────────────────────────────────────────────────
// AddDefaultRoute
// ─────────────────────────────────────────────────────────────────────────────

TEST(RoutingManager, AddDefaultRouteCallsCreateIpForwardEntry2) {
    FakeWin32::CallLog::Reset();
    NET_LUID luid = make_luid(0xABCD);

    RoutingManager mgr;
    auto result = mgr.AddDefaultRoute(luid);

    ASSERT_TRUE(result.has_value());
    auto& calls = FakeWin32::CallLog::Get();
    ASSERT_EQ(calls.CreateIpForwardEntry2.size(), 1u);
    EXPECT_EQ(calls.CreateIpForwardEntry2[0].row.InterfaceLuid.Value, 0xABCDu);
    EXPECT_EQ(calls.CreateIpForwardEntry2[0].row.DestinationPrefix.PrefixLength, 0u);
}

TEST(RoutingManager, AddDefaultRouteIsIdempotent) {
    FakeWin32::CallLog::Reset();
    NET_LUID luid = make_luid(0x1);

    RoutingManager mgr;
    ASSERT_TRUE(mgr.AddDefaultRoute(luid).has_value());
    ASSERT_TRUE(mgr.AddDefaultRoute(luid).has_value());

    // Second call must not reach the Win32 API
    EXPECT_EQ(FakeWin32::CallLog::Get().CreateIpForwardEntry2.size(), 1u);
}

TEST(RoutingManager, AddDefaultRoutePropagatesError) {
    FakeWin32::CallLog::Reset();
    FakeWin32::CallLog::Get().next_CreateIpForwardEntry2_result = 5u; // ERROR_ACCESS_DENIED

    RoutingManager mgr;
    auto result = mgr.AddDefaultRoute(make_luid(0x1));

    ASSERT_FALSE(result.has_value());
    EXPECT_EQ(static_cast<DWORD>(result.error().value()), 5u);
}

// ─────────────────────────────────────────────────────────────────────────────
// RemoveDefaultRoute
// ─────────────────────────────────────────────────────────────────────────────

TEST(RoutingManager, RemoveDefaultRouteCallsDeleteIpForwardEntry2) {
    FakeWin32::CallLog::Reset();
    NET_LUID luid = make_luid(0xBEEF);

    RoutingManager mgr;
    ASSERT_TRUE(mgr.AddDefaultRoute(luid).has_value());
    FakeWin32::CallLog::Reset();  // clear Add record; focus on Remove

    auto result = mgr.RemoveDefaultRoute(luid);

    ASSERT_TRUE(result.has_value());
    EXPECT_EQ(FakeWin32::CallLog::Get().DeleteIpForwardEntry2.size(), 1u);
}

TEST(RoutingManager, RemoveDefaultRouteIsIdempotentWhenNotInstalled) {
    FakeWin32::CallLog::Reset();

    RoutingManager mgr;
    ASSERT_TRUE(mgr.RemoveDefaultRoute(make_luid(0x1)).has_value());
    EXPECT_EQ(FakeWin32::CallLog::Get().DeleteIpForwardEntry2.size(), 0u);
}

TEST(RoutingManager, RemoveDefaultRoutePropagatesError) {
    FakeWin32::CallLog::Reset();

    RoutingManager mgr;
    ASSERT_TRUE(mgr.AddDefaultRoute(make_luid(0x1)).has_value());

    FakeWin32::CallLog::Get().next_DeleteIpForwardEntry2_result = 5u;
    auto result = mgr.RemoveDefaultRoute(make_luid(0x1));

    ASSERT_FALSE(result.has_value());
}

// ─────────────────────────────────────────────────────────────────────────────
// SetDnsServers / ClearDnsServers
// ─────────────────────────────────────────────────────────────────────────────

TEST(RoutingManager, SetDnsServersCallsSetInterfaceDnsSettings) {
    FakeWin32::CallLog::Reset();
    GUID expected_guid = make_guid(0xDEAD);
    FakeWin32::CallLog::Get().next_ConvertInterfaceLuidToGuid_guid = expected_guid;

    RoutingManager mgr;
    auto result = mgr.SetDnsServers(make_luid(0x42),
                                    {L"1.1.1.1", L"8.8.8.8"});

    ASSERT_TRUE(result.has_value());
    auto& calls = FakeWin32::CallLog::Get();
    ASSERT_EQ(calls.SetInterfaceDnsSettings.size(), 1u);
    EXPECT_EQ(calls.SetInterfaceDnsSettings[0].adapter_guid, expected_guid);
    EXPECT_EQ(calls.SetInterfaceDnsSettings[0].name_server, L"1.1.1.1 8.8.8.8");
    EXPECT_EQ(calls.SetInterfaceDnsSettings[0].settings.Flags, DNS_SETTING_NAMESERVER);
}

TEST(RoutingManager, SetDnsServersFailsWhenLuidConversionFails) {
    FakeWin32::CallLog::Reset();
    FakeWin32::CallLog::Get().next_ConvertInterfaceLuidToGuid_result = 87u; // ERROR_INVALID_PARAMETER

    RoutingManager mgr;
    auto result = mgr.SetDnsServers(make_luid(0x1), {L"1.1.1.1"});

    ASSERT_FALSE(result.has_value());
    EXPECT_EQ(FakeWin32::CallLog::Get().SetInterfaceDnsSettings.size(), 0u);
}

TEST(RoutingManager, ClearDnsServersPassesNullNameServer) {
    FakeWin32::CallLog::Reset();
    FakeWin32::CallLog::Get().next_ConvertInterfaceLuidToGuid_guid = make_guid(0xABBA);

    RoutingManager mgr;
    auto result = mgr.ClearDnsServers(make_luid(0x5));

    ASSERT_TRUE(result.has_value());
    auto& calls = FakeWin32::CallLog::Get();
    ASSERT_EQ(calls.SetInterfaceDnsSettings.size(), 1u);
    EXPECT_TRUE(calls.SetInterfaceDnsSettings[0].name_server.empty());
}
