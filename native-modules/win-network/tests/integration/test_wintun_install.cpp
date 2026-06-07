// Layer 2 integration test — requires elevation and the real Wintun driver.
// Guards itself with GTEST_SKIP when not running elevated.

#include <gtest/gtest.h>

#ifndef HOLY_BLOCKER_FAKE_WIN32
#  include <windows.h>
#  include <shlobj.h>  // IsUserAnAdministrator

TEST(WintunAdapter, InstallAndOpen) {
    if (!IsUserAnAdministrator()) {
        GTEST_SKIP() << "requires elevation";
    }
    // TODO: call WintunAdapter::Install and WintunAdapter::Open with the real
    // driver once step 2 is implemented.
    SUCCEED();
}

#else

TEST(WintunAdapter, SkippedInFakeWin32Mode) {
    GTEST_SKIP() << "integration tests require real Win32";
}

#endif
