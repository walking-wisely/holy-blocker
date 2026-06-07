// Layer 1 tests — fake_win32 shim, no elevation needed.
// Covers WintunAdapter registry persistence and handle lifecycle.

#include <gtest/gtest.h>
#include "wintun_adapter.h"
#include "recorder.h"

class WintunAdapterTest : public ::testing::Test {
protected:
    void SetUp() override {
        FakeWin32::CallLog::Reset();
        FakeWin32::ResetRegistry();
    }
};

// ──────────────────────────────────────────────────────────────────────────────
// Install
// ──────────────────────────────────────────────────────────────────────────────

TEST_F(WintunAdapterTest, Install_CallsCreateAdapterWithGivenName) {
    auto result = WintunAdapter::Install(L"HolyBlocker");
    ASSERT_TRUE(result.has_value());

    auto& log = FakeWin32::CallLog::Get();
    ASSERT_EQ(log.WintunCreateAdapter.size(), 1u);
    EXPECT_EQ(log.WintunCreateAdapter[0].name, L"HolyBlocker");

    result->Close();
}

TEST_F(WintunAdapterTest, Install_PassesTunnelTypeToCreateAdapter) {
    auto result = WintunAdapter::Install(L"HolyBlocker");
    ASSERT_TRUE(result.has_value());

    EXPECT_EQ(FakeWin32::CallLog::Get().WintunCreateAdapter[0].tunnel_type,
              L"HolyBlocker");

    result->Close();
}

TEST_F(WintunAdapterTest, Install_PersistsGuidInRegistry) {
    auto result = WintunAdapter::Install(L"HolyBlocker");
    ASSERT_TRUE(result.has_value());

    bool found = false;
    for (auto& call : FakeWin32::CallLog::Get().RegSetValue)
        if (call.value_name == L"AdapterGUID") found = true;
    EXPECT_TRUE(found) << "AdapterGUID not written to registry";

    result->Close();
}

TEST_F(WintunAdapterTest, Install_PersistsNameInRegistry) {
    auto result = WintunAdapter::Install(L"HolyBlocker");
    ASSERT_TRUE(result.has_value());

    bool found = false;
    for (auto& call : FakeWin32::CallLog::Get().RegSetValue)
        if (call.value_name == L"AdapterName") found = true;
    EXPECT_TRUE(found) << "AdapterName not written to registry";

    result->Close();
}

// ──────────────────────────────────────────────────────────────────────────────
// Open
// ──────────────────────────────────────────────────────────────────────────────

TEST_F(WintunAdapterTest, Open_AfterInstall_CallsOpenAdapterWithStoredName) {
    // Install to populate the registry, then close the adapter.
    {
        auto install = WintunAdapter::Install(L"HolyBlocker");
        ASSERT_TRUE(install.has_value());
        install->Close();
    }
    // Reset the call log only — keep the registry intact.
    FakeWin32::CallLog::Reset();

    auto result = WintunAdapter::Open();
    ASSERT_TRUE(result.has_value());

    auto& log = FakeWin32::CallLog::Get();
    ASSERT_EQ(log.WintunOpenAdapter.size(), 1u);
    EXPECT_EQ(log.WintunOpenAdapter[0].name, L"HolyBlocker");

    result->Close();
}

TEST_F(WintunAdapterTest, Open_WhenRegistryEmpty_FallsBackToInstall) {
    // Registry is empty (SetUp cleared it).
    auto result = WintunAdapter::Open();
    ASSERT_TRUE(result.has_value());

    // Should have installed (created) an adapter, not tried to open one.
    auto& log = FakeWin32::CallLog::Get();
    EXPECT_EQ(log.WintunCreateAdapter.size(), 1u);
    EXPECT_EQ(log.WintunOpenAdapter.size(), 0u);

    result->Close();
}

// ──────────────────────────────────────────────────────────────────────────────
// StartSession / Close
// ──────────────────────────────────────────────────────────────────────────────

TEST_F(WintunAdapterTest, StartSession_ReturnsNonNull) {
    auto result = WintunAdapter::Install(L"HolyBlocker");
    ASSERT_TRUE(result.has_value());

    WINTUN_SESSION_HANDLE session = result->StartSession();
    EXPECT_NE(session, nullptr);

    result->Close();
}

TEST_F(WintunAdapterTest, Close_CanBeCalledTwiceWithoutCrash) {
    auto result = WintunAdapter::Install(L"HolyBlocker");
    ASSERT_TRUE(result.has_value());
    result->StartSession();
    result->Close();
    result->Close(); // second call must be a safe no-op
}

// ──────────────────────────────────────────────────────────────────────────────
// Luid
// ──────────────────────────────────────────────────────────────────────────────

TEST_F(WintunAdapterTest, Luid_ReturnsNonZeroAfterInstall) {
    auto result = WintunAdapter::Install(L"HolyBlocker");
    ASSERT_TRUE(result.has_value());

    NET_LUID luid = result->Luid();
    EXPECT_NE(luid.Value, 0u);

    result->Close();
}

// ──────────────────────────────────────────────────────────────────────────────
// Uninstall
// ──────────────────────────────────────────────────────────────────────────────

TEST_F(WintunAdapterTest, Uninstall_DeletesAdapter) {
    {
        auto result = WintunAdapter::Install(L"HolyBlocker");
        ASSERT_TRUE(result.has_value());
        result->Close();
    }
    FakeWin32::CallLog::Reset();

    WintunAdapter::Uninstall();

    EXPECT_EQ(FakeWin32::CallLog::Get().WintunDeleteAdapter.size(), 1u);
}

TEST_F(WintunAdapterTest, Uninstall_ClearsRegistry) {
    {
        auto result = WintunAdapter::Install(L"HolyBlocker");
        ASSERT_TRUE(result.has_value());
        result->Close();
    }

    WintunAdapter::Uninstall();

    // After Uninstall, Open() should fall back to Install (registry is gone).
    FakeWin32::CallLog::Reset();
    auto result = WintunAdapter::Open();
    ASSERT_TRUE(result.has_value());
    EXPECT_EQ(FakeWin32::CallLog::Get().WintunCreateAdapter.size(), 1u)
        << "registry should be empty after Uninstall";
    result->Close();
}
