#include <windows.h>

#include <iostream>
#include <iterator>
#include <string>

namespace {

void Log(const std::wstring& message) {
  std::wcout << L"[holy-blocker-win-daemon] " << message << std::endl;
}

std::wstring EventName(DWORD event) {
  switch (event) {
    case EVENT_SYSTEM_FOREGROUND:
      return L"foreground";
    case EVENT_OBJECT_LOCATIONCHANGE:
      return L"location-change";
    default:
      return L"event-" + std::to_wstring(event);
  }
}

void CALLBACK HandleWinEvent(
    HWINEVENTHOOK,
    DWORD event,
    HWND hwnd,
    LONG object_id,
    LONG child_id,
    DWORD,
    DWORD) {
  if (hwnd == nullptr || object_id != OBJID_WINDOW || child_id != CHILDID_SELF) {
    return;
  }

  wchar_t title[256] = {};
  GetWindowTextW(hwnd, title, static_cast<int>(std::size(title)));

  RECT bounds = {};
  GetWindowRect(hwnd, &bounds);

  Log(EventName(event) + L": \"" + title + L"\" [" +
      std::to_wstring(bounds.left) + L"," + std::to_wstring(bounds.top) + L" " +
      std::to_wstring(bounds.right - bounds.left) + L"x" +
      std::to_wstring(bounds.bottom - bounds.top) + L"]");
}

HWINEVENTHOOK RegisterHook(DWORD event_min, DWORD event_max) {
  return SetWinEventHook(
      event_min,
      event_max,
      nullptr,
      HandleWinEvent,
      0,
      0,
      WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
}

}  // namespace

int main() {
  Log(L"starting");

  const HWINEVENTHOOK foreground_hook =
      RegisterHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND);
  const HWINEVENTHOOK location_hook =
      RegisterHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE);

  if (foreground_hook == nullptr || location_hook == nullptr) {
    Log(L"failed to register WinEvent hooks");
    return 1;
  }

  Log(L"hooks registered");

  MSG message;
  while (GetMessageW(&message, nullptr, 0, 0) > 0) {
    TranslateMessage(&message);
    DispatchMessageW(&message);
  }

  UnhookWinEvent(foreground_hook);
  UnhookWinEvent(location_hook);
  Log(L"stopped");

  return 0;
}
