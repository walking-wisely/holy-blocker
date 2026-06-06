// Layer 1 tests — fake_win32 shim, no elevation needed.
// Covers WintunAdapter registry persistence and handle lifecycle.
// Real tests will be added in step 2.

#include <gtest/gtest.h>
#include "recorder.h"

TEST(WintunAdapter, FakeWintunCreateAdapterRecordsName) {
    FakeWin32::CallLog::Reset();

    // Exercise the fake directly to verify the shim works end-to-end.
    WINTUN_ADAPTER_HANDLE h =
        FakeWintunCreateAdapter(L"HolyBlocker", L"HolyBlocker", nullptr);
    ASSERT_NE(h, nullptr);

    auto& log = FakeWin32::CallLog::Get();
    ASSERT_EQ(log.WintunCreateAdapter.size(), 1u);
    EXPECT_EQ(log.WintunCreateAdapter[0].name, L"HolyBlocker");

    FakeWintunCloseAdapter(h);
}

TEST(WintunAdapter, PlaceholderPassesUntilStep2) {
    // Real WintunAdapter tests are added in step 2.
    SUCCEED();
}
