#pragma once

#include <string>

// TODO(step 6): install/uninstall helpers called by the elevated installer.

class InstallerActions {
public:
    static bool InstallService(const std::wstring& binary_path);
    static bool UninstallService();
    static bool CopyWintunDll(const std::wstring& source_dir);
    static bool RemoveWintunDll();
};
