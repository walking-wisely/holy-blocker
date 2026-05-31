cmake_minimum_required(VERSION 3.24)

project(holy_blocker_win_daemon LANGUAGES CXX)

add_executable(holy-blocker-win-daemon
  src/main.cpp
)

target_compile_features(holy-blocker-win-daemon PRIVATE cxx_std_20)

if(MSVC)
  target_compile_options(holy-blocker-win-daemon PRIVATE /W4 /permissive-)
else()
  target_compile_options(holy-blocker-win-daemon PRIVATE -Wall -Wextra -Wpedantic)
endif()
