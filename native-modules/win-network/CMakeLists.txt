cmake_minimum_required(VERSION 3.24)

project(holy_blocker_win_network LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# ──────────────────────────────────────────────────────────────────────────────
# Options
# ──────────────────────────────────────────────────────────────────────────────
option(HOLY_BLOCKER_FAKE_WIN32       "Use fake Win32 shim (no elevation needed)" OFF)
option(HOLY_BLOCKER_INTEGRATION_TESTS "Build admin-required integration tests"   OFF)

# ──────────────────────────────────────────────────────────────────────────────
# GoogleTest via FetchContent
# ──────────────────────────────────────────────────────────────────────────────
include(FetchContent)
FetchContent_Declare(
  googletest
  GIT_REPOSITORY https://github.com/google/googletest.git
  GIT_TAG        v1.14.0
)
set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(googletest)

# ──────────────────────────────────────────────────────────────────────────────
# Compiler flags
# ──────────────────────────────────────────────────────────────────────────────
add_compile_options(
  $<$<CXX_COMPILER_ID:MSVC>:/W4>
  $<$<CXX_COMPILER_ID:MSVC>:/WX>
  $<$<CXX_COMPILER_ID:MSVC>:/permissive->
  $<$<NOT:$<CXX_COMPILER_ID:MSVC>>:-Wall>
  $<$<NOT:$<CXX_COMPILER_ID:MSVC>>:-Wextra>
  $<$<NOT:$<CXX_COMPILER_ID:MSVC>>:-Wpedantic>
)

# Minimum Windows SDK: 10.0.19041.0 (required for SetInterfaceDnsSettings)
add_compile_definitions(
  _WIN32_WINNT=0x0A00
  WINVER=0x0A00
  NTDDI_VERSION=0x0A000008   # 20H1 = 0x0A000008
)

# ──────────────────────────────────────────────────────────────────────────────
# fake_win32 shim static library
# ──────────────────────────────────────────────────────────────────────────────
add_library(fake_win32 STATIC
  fake_win32/recorder.cpp
  fake_win32/fake_wintun.cpp
  fake_win32/fake_iphelper.cpp
  fake_win32/fake_registry.cpp
  fake_win32/fake_scm.cpp
)

target_include_directories(fake_win32 PUBLIC
  fake_win32
)

target_compile_features(fake_win32 PUBLIC cxx_std_23)

# fake_win32 must NOT pull in real windows.h — it IS the replacement
target_compile_definitions(fake_win32 PUBLIC HOLY_BLOCKER_FAKE_WIN32)

# ──────────────────────────────────────────────────────────────────────────────
# Service source files (shared between executable and tests)
# ──────────────────────────────────────────────────────────────────────────────
set(SVC_SOURCES
  src/wintun_adapter.cpp
  src/routing.cpp
  src/ipc_server.cpp
  src/service_host.cpp
  src/installer_actions.cpp
)

# ──────────────────────────────────────────────────────────────────────────────
# holy_blocker_net_svc — the service executable
# ──────────────────────────────────────────────────────────────────────────────
add_executable(holy_blocker_net_svc
  src/main.cpp
  ${SVC_SOURCES}
)

target_include_directories(holy_blocker_net_svc PRIVATE
  src
  vendor/wintun
)

target_link_libraries(holy_blocker_net_svc PRIVATE
  iphlpapi
  ws2_32
)

# ──────────────────────────────────────────────────────────────────────────────
# net_svc_tests — Layer 0 + Layer 1 (no elevation needed, runs in CI)
# ──────────────────────────────────────────────────────────────────────────────
add_executable(net_svc_tests
  tests/test_logic.cpp
  tests/test_routing.cpp
  tests/test_ipc_server.cpp
  tests/test_wintun_adapter.cpp
  ${SVC_SOURCES}
)

target_include_directories(net_svc_tests PRIVATE
  src
  vendor/wintun
)

target_link_libraries(net_svc_tests PRIVATE
  fake_win32
  GTest::gtest_main
)

target_compile_definitions(net_svc_tests PRIVATE HOLY_BLOCKER_FAKE_WIN32)

include(GoogleTest)
gtest_discover_tests(net_svc_tests)

# ──────────────────────────────────────────────────────────────────────────────
# net_svc_integration_tests — Layer 2 (admin-required, not in standard CI)
# ──────────────────────────────────────────────────────────────────────────────
if(HOLY_BLOCKER_INTEGRATION_TESTS)
  add_executable(net_svc_integration_tests
    tests/integration/test_wintun_install.cpp
    tests/integration/test_routing_live.cpp
    ${SVC_SOURCES}
  )

  target_include_directories(net_svc_integration_tests PRIVATE
    src
    vendor/wintun
  )

  target_link_libraries(net_svc_integration_tests PRIVATE
    GTest::gtest_main
    iphlpapi
    ws2_32
  )
endif()
