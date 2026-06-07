#pragma once

#include <string>

// Command handler interface — implemented by the service, tested independently.
struct CommandHandler {
    virtual std::string OnStart()  = 0;
    virtual std::string OnStop()   = 0;
    virtual std::string OnStatus() = 0;
    virtual std::string OnReloadRules(const std::string& path) = 0;
    virtual ~CommandHandler() = default;
};

class IpcServer {
public:
    explicit IpcServer(CommandHandler& handler);
    ~IpcServer();

    // Parses a newline-delimited JSON command string and returns the response.
    // Pure logic; does not open a real pipe.
    std::string Dispatch(const std::string& json_command);

    // Blocks, serving connections on \\.\pipe\HolyBlockerNetSvc until Stop()
    // is called from another thread.
    void Run();

    // Signals Run() to exit. Safe to call from any thread.
    void Stop();

private:
    CommandHandler& handler_;
    void*           stop_event_{nullptr};  // HANDLE, typed as void* to keep Win32 out of header
};
