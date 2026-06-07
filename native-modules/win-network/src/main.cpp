// Entry point for holy_blocker_net_svc.exe.
// Handles two modes:
//   No args        → StartServiceCtrlDispatcher (normal SCM launch)
//   --install      → register the service with the SCM (requires elevation)
//   --uninstall    → stop and delete the service (requires elevation)

#include "service_host.h"
#include "installer_actions.h"

#ifndef HOLY_BLOCKER_FAKE_WIN32
#  include <windows.h>
#endif

#include <string>
#include <string_view>

int wmain(int argc, wchar_t* argv[]) {
    if (argc >= 2) {
        std::wstring_view arg = argv[1];
        if (arg == L"--install") {
            wchar_t path[1024]{};
#ifndef HOLY_BLOCKER_FAKE_WIN32
            GetModuleFileNameW(nullptr, path, static_cast<DWORD>(std::size(path)));
#endif
            return InstallerActions::InstallService(path) ? 0 : 1;
        }
        if (arg == L"--uninstall") {
            return InstallerActions::UninstallService() ? 0 : 1;
        }
    }

    ServiceHost::Run();
    return 0;
}
