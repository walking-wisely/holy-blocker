// Layer 0/1 tests — IpcServer command parsing and response serialization.
// Dispatch() is pure: no pipe I/O, no Win32 calls.

#include <gtest/gtest.h>
#include "ipc_server.h"
#include "recorder.h"

// ──────────────────────────────────────────────────────────────────────────────
// Stub handler for tests
// ──────────────────────────────────────────────────────────────────────────────
struct StubHandler : CommandHandler {
    std::string start_response  = R"({"ok":true})";
    std::string stop_response   = R"({"ok":true})";
    std::string status_response = R"({"state":"stopped"})";
    std::string reload_response = R"({"ok":true})";

    std::string last_reload_path;
    int         start_calls  = 0;
    int         stop_calls   = 0;
    int         status_calls = 0;
    int         reload_calls = 0;

    std::string OnStart()  override { ++start_calls;  return start_response; }
    std::string OnStop()   override { ++stop_calls;   return stop_response;  }
    std::string OnStatus() override { ++status_calls; return status_response; }
    std::string OnReloadRules(const std::string& path) override {
        ++reload_calls;
        last_reload_path = path;
        return reload_response;
    }
};

class IpcServerTest : public ::testing::Test {
protected:
    StubHandler handler;
    IpcServer   server{handler};

    void SetUp() override { FakeWin32::CallLog::Reset(); }
};

// ──────────────────────────────────────────────────────────────────────────────
// Happy-path dispatch
// ──────────────────────────────────────────────────────────────────────────────

TEST_F(IpcServerTest, DispatchStart) {
    auto resp = server.Dispatch(R"({"cmd":"start"})");
    EXPECT_EQ(resp, R"({"ok":true})");
    EXPECT_EQ(handler.start_calls, 1);
}

TEST_F(IpcServerTest, DispatchStop) {
    auto resp = server.Dispatch(R"({"cmd":"stop"})");
    EXPECT_EQ(resp, R"({"ok":true})");
    EXPECT_EQ(handler.stop_calls, 1);
}

TEST_F(IpcServerTest, DispatchStatus) {
    auto resp = server.Dispatch(R"({"cmd":"status"})");
    EXPECT_EQ(resp, R"({"state":"stopped"})");
    EXPECT_EQ(handler.status_calls, 1);
}

TEST_F(IpcServerTest, DispatchReloadRules) {
    auto resp = server.Dispatch(R"({"cmd":"reload_rules","path":"C:\\rules\\block.json"})");
    EXPECT_EQ(resp, R"({"ok":true})");
    EXPECT_EQ(handler.reload_calls, 1);
    EXPECT_EQ(handler.last_reload_path, "C:\\rules\\block.json");
}

// ──────────────────────────────────────────────────────────────────────────────
// Error cases
// ──────────────────────────────────────────────────────────────────────────────

TEST_F(IpcServerTest, MissingCmdField) {
    auto resp = server.Dispatch(R"({"action":"start"})");
    EXPECT_NE(resp.find("missing cmd"), std::string::npos);
    EXPECT_EQ(handler.start_calls, 0);
}

TEST_F(IpcServerTest, UnknownCommand) {
    auto resp = server.Dispatch(R"({"cmd":"reboot"})");
    EXPECT_NE(resp.find("unknown command"), std::string::npos);
}

TEST_F(IpcServerTest, ReloadRulesMissingPath) {
    auto resp = server.Dispatch(R"({"cmd":"reload_rules"})");
    EXPECT_NE(resp.find("missing path"), std::string::npos);
    EXPECT_EQ(handler.reload_calls, 0);
}

TEST_F(IpcServerTest, EmptyInput) {
    auto resp = server.Dispatch("");
    EXPECT_NE(resp.find("missing cmd"), std::string::npos);
}

TEST_F(IpcServerTest, MalformedJson) {
    auto resp = server.Dispatch("{not json}");
    EXPECT_NE(resp.find("missing cmd"), std::string::npos);
}

// ──────────────────────────────────────────────────────────────────────────────
// JSON field extraction edge cases
// ──────────────────────────────────────────────────────────────────────────────

TEST_F(IpcServerTest, ExtraWhitespaceAroundColon) {
    // Spaces between key and value should still parse.
    auto resp = server.Dispatch(R"({"cmd" : "start"})");
    EXPECT_EQ(handler.start_calls, 1);
}

TEST_F(IpcServerTest, PathWithEscapedBackslash) {
    auto resp = server.Dispatch(R"({"cmd":"reload_rules","path":"C:\\data\\rules.json"})");
    EXPECT_EQ(handler.last_reload_path, "C:\\data\\rules.json");
}

TEST_F(IpcServerTest, HandlerResponsePassedThrough) {
    handler.status_response = R"({"state":"running","error":""})";
    auto resp = server.Dispatch(R"({"cmd":"status"})");
    EXPECT_EQ(resp, R"({"state":"running","error":""})");
}

TEST_F(IpcServerTest, MultipleDispatchCallsAreIndependent) {
    server.Dispatch(R"({"cmd":"start"})");
    server.Dispatch(R"({"cmd":"start"})");
    server.Dispatch(R"({"cmd":"stop"})");
    EXPECT_EQ(handler.start_calls, 2);
    EXPECT_EQ(handler.stop_calls, 1);
}
