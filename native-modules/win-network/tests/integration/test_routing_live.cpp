// Layer 2 integration test — requires elevation and a real routing table.
// Guards itself with GTEST_SKIP when not running elevated.

#include <gtest/gtest.h>

#ifndef HOLY_BLOCKER_FAKE_WIN32
#  include <windows.h>
#  include <shlobj.h>

TEST(RoutingManager, AddAndRemoveDefaultRoute) {
    if (!IsUserAnAdministrator()) {
        GTEST_SKIP() << "requires elevation";
    }
    // TODO: exercise RoutingManager::AddDefaultRoute and RemoveDefaultRoute
    // with a real Wintun adapter LUID once steps 2 and 3 are implemented.
    SUCCEED();
}

#else

TEST(RoutingManager, SkippedInFakeWin32Mode) {
    GTEST_SKIP() << "integration tests require real Win32";
}

#endif
