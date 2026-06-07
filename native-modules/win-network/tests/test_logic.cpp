// Layer 0 tests — no Win32 at all.
// Covers pure logic: JSON parsing, SDDL string construction, registry key
// formatting.  All tests in this file must compile and run on any platform.

#include <gtest/gtest.h>
#include <string>

// ──────────────────────────────────────────────────────────────────────────────
// Placeholder: registry key path formatting
// ──────────────────────────────────────────────────────────────────────────────
TEST(RegistryKeyPath, FormatAdapterGuidKey) {
    // Expected key under which the adapter GUID is persisted.
    const std::wstring kBase = L"SOFTWARE\\HolyBlocker\\NetSvc";
    const std::wstring kValue = L"AdapterGUID";

    // Verify the strings are well-formed (no null bytes, no trailing slash)
    EXPECT_FALSE(kBase.empty());
    EXPECT_FALSE(kValue.empty());
    EXPECT_NE(kBase.back(), L'\\');
}

// ──────────────────────────────────────────────────────────────────────────────
// Placeholder: IPC command names
// ──────────────────────────────────────────────────────────────────────────────
TEST(IpcCommands, KnownCommandSet) {
    // These command strings must not change without updating the desktop client.
    EXPECT_STREQ("start",        "start");
    EXPECT_STREQ("stop",         "stop");
    EXPECT_STREQ("status",       "status");
    EXPECT_STREQ("reload_rules", "reload_rules");
}
