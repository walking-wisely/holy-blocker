#include "win32_api.h"
#include "recorder.h"
#include <map>
#include <vector>
#include <string>

// In-memory registry: path → (value_name → raw bytes)
static std::map<std::wstring, std::map<std::wstring, std::vector<uint8_t>>> g_reg;

namespace FakeWin32 {
void ResetRegistry() { g_reg.clear(); }
} // namespace FakeWin32

// Fake HKEY: we use the key path as the identity by encoding a pointer to a
// heap-allocated string.  Tests don't need to inspect handles — they check the
// CallLog instead.
static HKEY MakeHKey(const std::wstring& path) {
    auto* s = new std::wstring(path);
    return reinterpret_cast<HKEY>(s);
}
static const std::wstring& PathOf(HKEY hKey) {
    return *reinterpret_cast<const std::wstring*>(hKey);
}

LONG RegCreateKeyExW(HKEY hKey, LPCWSTR lpSubKey, DWORD /*Reserved*/,
                     LPWSTR /*lpClass*/, DWORD /*dwOptions*/,
                     DWORD /*samDesired*/, void* /*lpSecurityAttributes*/,
                     HKEY* phkResult, DWORD* lpdwDisposition) {
    std::wstring full = (hKey == HKEY_LOCAL_MACHINE ? L"HKLM" : PathOf(hKey));
    full += L'\\';
    full += lpSubKey;
    *phkResult = MakeHKey(full);
    if (lpdwDisposition) *lpdwDisposition = 0;
    return 0;  // ERROR_SUCCESS
}

LONG RegSetValueExW(HKEY hKey, LPCWSTR lpValueName, DWORD /*Reserved*/,
                    DWORD dwType, const BYTE* lpData, DWORD cbData) {
    const std::wstring& path = PathOf(hKey);
    std::wstring name = lpValueName ? lpValueName : L"";
    std::vector<uint8_t> data(lpData, lpData + cbData);
    g_reg[path][name] = data;

    FakeWin32::RegSetValueCall rec{};
    rec.key_path   = path;
    rec.value_name = name;
    rec.type       = dwType;
    rec.data       = data;
    FakeWin32::CallLog::Get().RegSetValue.push_back(std::move(rec));
    return 0;
}

LONG RegGetValueW(HKEY hKey, LPCWSTR lpSubKey, LPCWSTR lpValue,
                  DWORD /*dwFlags*/, DWORD* pdwType, void* pvData,
                  DWORD* pcbData) {
    std::wstring path = (hKey == HKEY_LOCAL_MACHINE ? L"HKLM" : PathOf(hKey));
    if (lpSubKey && *lpSubKey) {
        path += L'\\';
        path += lpSubKey;
    }
    std::wstring name = lpValue ? lpValue : L"";

    FakeWin32::RegGetValueCall rec{path, name};
    FakeWin32::CallLog::Get().RegGetValue.push_back(rec);

    auto kit = g_reg.find(path);
    if (kit == g_reg.end()) return ERROR_NOT_FOUND;
    auto vit = kit->second.find(name);
    if (vit == kit->second.end()) return ERROR_NOT_FOUND;

    const auto& data = vit->second;
    if (pdwType) *pdwType = REG_BINARY;
    if (pcbData) {
        if (pvData && *pcbData >= static_cast<DWORD>(data.size())) {
            std::memcpy(pvData, data.data(), data.size());
        }
        *pcbData = static_cast<DWORD>(data.size());
    }
    return 0;
}

LONG RegDeleteKeyExW(HKEY hKey, LPCWSTR lpSubKey, DWORD /*samDesired*/,
                     DWORD /*Reserved*/) {
    std::wstring path = (hKey == HKEY_LOCAL_MACHINE ? L"HKLM" : PathOf(hKey));
    if (lpSubKey && *lpSubKey) {
        path += L'\\';
        path += lpSubKey;
    }
    g_reg.erase(path);
    return 0;
}

LONG RegCloseKey(HKEY hKey) {
    if (hKey && hKey != HKEY_LOCAL_MACHINE) {
        delete reinterpret_cast<std::wstring*>(hKey);
    }
    return 0;
}
