cmake_minimum_required(VERSION 3.25)
project(hello C)

find_package(libevent REQUIRED)

add_executable(${PROJECT_NAME} src/main.c)
target_link_libraries(${PROJECT_NAME} libevent::libevent)

