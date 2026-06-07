#include "recorder.h"

namespace FakeWin32 {

static CallLog g_log;

CallLog& CallLog::Get() {
    return g_log;
}

void CallLog::Reset() {
    g_log = {};
}

} // namespace FakeWin32
