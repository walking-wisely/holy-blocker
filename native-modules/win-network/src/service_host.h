#pragma once

// TODO(step 5): Windows Service Control Manager integration.
// Declared here so other modules can forward-declare it.

class ServiceHost {
public:
    // Entry point called by StartServiceCtrlDispatcher.
    static void Run();
};
