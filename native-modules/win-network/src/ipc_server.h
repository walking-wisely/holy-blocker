#pragma once

#include <functional>
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

    // Parses a newline-delimited JSON command string and returns the response.
    // Pure logic; does not open a real pipe.
    std::string Dispatch(const std::string& json_command);

    // TODO(step 4): Run() — opens the named pipe and serves connections.

private:
    CommandHandler& handler_;
};
