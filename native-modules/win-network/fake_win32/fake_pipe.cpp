#include "win32_api.h"
#include "recorder.h"

#include <cstdlib>
#include <cstring>

// ──────────────────────────────────────────────────────────────────────────────
// Fake thread-local "last error" state
// ──────────────────────────────────────────────────────────────────────────────
static thread_local DWORD t_last_error = 0;

DWORD GetLastError() { return t_last_error; }

static void set_last_error(DWORD e) { t_last_error = e; }

// ──────────────────────────────────────────────────────────────────────────────
// Named pipe stubs
//
// Under the shim, CreateNamedPipeW returns a non-null sentinel handle.
// ConnectNamedPipe immediately "succeeds" (returns TRUE) so tests that call
// Run() don't block.  The unit tests only call Dispatch() directly — these
// stubs exist solely to make the service sources compile and link.
// ──────────────────────────────────────────────────────────────────────────────

static HANDLE kFakePipeHandle()  { return reinterpret_cast<HANDLE>(0x1001LL); }
static HANDLE kFakeEventHandle() { return reinterpret_cast<HANDLE>(0x1002LL); }

HANDLE CreateNamedPipeW(LPCWSTR /*lpName*/, DWORD /*dwOpenMode*/,
                        DWORD /*dwPipeMode*/, DWORD /*nMaxInstances*/,
                        DWORD /*nOutBufferSize*/, DWORD /*nInBufferSize*/,
                        DWORD /*nDefaultTimeOut*/,
                        LPSECURITY_ATTRIBUTES /*lpSecurityAttributes*/) {
    auto& log = FakeWin32::CallLog::Get();
    if (!log.next_CreateNamedPipe_ok) {
        set_last_error(5); // ERROR_ACCESS_DENIED — arbitrary failure sentinel
        return INVALID_HANDLE_VALUE;
    }
    set_last_error(0);
    return kFakePipeHandle();
}

BOOL ConnectNamedPipe(HANDLE /*hNamedPipe*/, LPOVERLAPPED /*lpOverlapped*/) {
    // Fake: signal immediate connection (no async wait needed in unit tests).
    set_last_error(ERROR_PIPE_CONNECTED);
    return FALSE;  // returns FALSE + ERROR_PIPE_CONNECTED when already connected
}

BOOL DisconnectNamedPipe(HANDLE /*hNamedPipe*/) {
    return TRUE;
}

BOOL ReadFile(HANDLE /*hFile*/, void* /*lpBuffer*/,
              DWORD /*nNumberOfBytesToRead*/, DWORD* lpNumberOfBytesRead,
              LPOVERLAPPED /*lpOverlapped*/) {
    // Fake: return 0 bytes so the caller gracefully closes the connection.
    if (lpNumberOfBytesRead) *lpNumberOfBytesRead = 0;
    set_last_error(ERROR_BROKEN_PIPE);
    return FALSE;
}

BOOL WriteFile(HANDLE /*hFile*/, const void* /*lpBuffer*/,
               DWORD nNumberOfBytesToWrite, DWORD* lpNumberOfBytesWritten,
               LPOVERLAPPED /*lpOverlapped*/) {
    if (lpNumberOfBytesWritten) *lpNumberOfBytesWritten = nNumberOfBytesToWrite;
    set_last_error(0);
    return TRUE;
}

BOOL CloseHandle(HANDLE /*hObject*/) {
    return TRUE;
}

HANDLE CreateEventW(LPSECURITY_ATTRIBUTES /*lpEventAttributes*/,
                    BOOL /*bManualReset*/, BOOL /*bInitialState*/,
                    LPCWSTR /*lpName*/) {
    return kFakeEventHandle();
}

BOOL SetEvent(HANDLE /*hEvent*/)   { return TRUE; }
BOOL ResetEvent(HANDLE /*hEvent*/) { return TRUE; }

DWORD WaitForSingleObject(HANDLE /*hHandle*/, DWORD /*dwMilliseconds*/) {
    // Return WAIT_OBJECT_0 so the fake stop signal appears signalled.
    return WAIT_OBJECT_0;
}

DWORD WaitForMultipleObjects(DWORD /*nCount*/, const HANDLE* /*lpHandles*/,
                              BOOL /*bWaitAll*/, DWORD /*dwMilliseconds*/) {
    // Index 0 = stop event; return it as signalled so Run() exits immediately.
    return WAIT_OBJECT_0;
}

BOOL GetOverlappedResult(HANDLE /*hFile*/, LPOVERLAPPED /*lpOverlapped*/,
                         DWORD* lpNumberOfBytesTransferred, BOOL /*bWait*/) {
    if (lpNumberOfBytesTransferred) *lpNumberOfBytesTransferred = 0;
    return TRUE;
}

BOOL ConvertStringSecurityDescriptorToSecurityDescriptorW(
    LPCWSTR /*StringSecurityDescriptor*/, DWORD /*StringSDRevision*/,
    PSECURITY_DESCRIPTOR* SecurityDescriptor,
    DWORD* /*SecurityDescriptorSize*/) {
    // Return a non-null placeholder so callers can pass it to CreateNamedPipeW.
    if (SecurityDescriptor) *SecurityDescriptor = reinterpret_cast<PSECURITY_DESCRIPTOR>(0x2001LL);
    return TRUE;
}

void* LocalFree(void* /*hMem*/) { return nullptr; }
