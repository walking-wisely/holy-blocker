#pragma once

#ifdef HOLY_BLOCKER_FAKE_WIN32
#  include "win32_api.h"
#else
#  include <windows.h>
#  include "wintun.h"
#endif

#include <expected>
#include <string>
#include <string_view>
#include <system_error>

class WintunAdapter {
public:
    static std::expected<WintunAdapter, std::error_code>
    Install(std::wstring_view name);

    static std::expected<WintunAdapter, std::error_code> Open();

    WINTUN_SESSION_HANDLE StartSession();
    void                  Close();
    static void           Uninstall();

    NET_LUID Luid() const;

private:
    explicit WintunAdapter(WINTUN_ADAPTER_HANDLE handle, GUID guid)
        : handle_(handle), guid_(guid) {}

    WINTUN_ADAPTER_HANDLE  handle_{};
    WINTUN_SESSION_HANDLE  session_{};
    GUID                   guid_{};
};
