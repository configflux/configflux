cmake_minimum_required(VERSION 3.16)
project(configflux_ros2_sdk LANGUAGES C CXX)

find_package(ament_cmake REQUIRED)
find_package(rclcpp REQUIRED)
find_package(rclcpp_action REQUIRED)
find_package(rcl_interfaces REQUIRED)
find_package(rosidl_default_generators REQUIRED)

set(configflux_ros2_interface_files
  "msg/RuntimeControlResult.msg"
  "msg/PullUpdateWrite.msg"
  "srv/CommitConfiguration.srv"
  "srv/RollbackDirty.srv"
  "srv/CheckForUpdates.srv"
  "srv/GetSyncStatus.srv"
  "srv/ExportPendingSyncBundle.srv"
  "srv/PushAuditEvents.srv"
  "action/PullUpdates.action"
)

rosidl_generate_interfaces(${PROJECT_NAME}
  ${configflux_ros2_interface_files}
)
rosidl_get_typesupport_target(cpp_typesupport_target ${PROJECT_NAME} "rosidl_typesupport_cpp")

add_library(configflux_ros2_runtime_parameter_path STATIC
  src/runtime_parameter_path.cc
)
target_include_directories(configflux_ros2_runtime_parameter_path
  PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(configflux_ros2_runtime_parameter_path PUBLIC cxx_std_20)

add_library(configflux_ros2_runtime_parameter_adapter STATIC
  src/runtime_parameter_adapter.cc
)
target_include_directories(configflux_ros2_runtime_parameter_adapter
  PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../../../cpp/include>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(configflux_ros2_runtime_parameter_adapter PUBLIC cxx_std_20)
ament_target_dependencies(configflux_ros2_runtime_parameter_adapter rclcpp rcl_interfaces)
target_link_libraries(configflux_ros2_runtime_parameter_adapter
  configflux_ros2_runtime_parameter_path
)

add_library(configflux_ros2_runtime_parameter_node_adapter STATIC
  src/runtime_parameter_node_adapter.cc
)
target_include_directories(configflux_ros2_runtime_parameter_node_adapter
  PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../../../cpp/include>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(configflux_ros2_runtime_parameter_node_adapter PUBLIC cxx_std_20)
ament_target_dependencies(configflux_ros2_runtime_parameter_node_adapter rclcpp rcl_interfaces)
target_link_libraries(configflux_ros2_runtime_parameter_node_adapter
  configflux_ros2_runtime_parameter_adapter
)

add_library(configflux_ros2_runtime_control_adapter STATIC
  src/runtime_control_adapter.cc
)
target_include_directories(configflux_ros2_runtime_control_adapter
  PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../../../cpp/include>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(configflux_ros2_runtime_control_adapter PUBLIC cxx_std_20)
ament_target_dependencies(configflux_ros2_runtime_control_adapter rclcpp)
target_link_libraries(configflux_ros2_runtime_control_adapter
  configflux_ros2_runtime_parameter_path
)

add_library(configflux_ros2_runtime_control_node_adapter STATIC
  src/runtime_control_node_adapter.cc
)
target_include_directories(configflux_ros2_runtime_control_node_adapter
  PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../../../cpp/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_BINARY_DIR}/rosidl_generator_cpp>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(configflux_ros2_runtime_control_node_adapter PUBLIC cxx_std_20)
ament_target_dependencies(configflux_ros2_runtime_control_node_adapter
  rclcpp
  rclcpp_action
)
target_link_libraries(configflux_ros2_runtime_control_node_adapter
  configflux_ros2_runtime_control_adapter
  "${cpp_typesupport_target}"
)
add_dependencies(configflux_ros2_runtime_control_node_adapter
  ${PROJECT_NAME}__rosidl_generator_cpp
)

add_library(configflux_ros2_runtime_lifecycle_mode STATIC
  src/runtime_lifecycle_mode.cc
)
target_include_directories(configflux_ros2_runtime_lifecycle_mode
  PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../../../cpp/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_BINARY_DIR}/rosidl_generator_cpp>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(configflux_ros2_runtime_lifecycle_mode PUBLIC cxx_std_20)
ament_target_dependencies(configflux_ros2_runtime_lifecycle_mode
  rclcpp
  rclcpp_action
  rcl_interfaces
)
target_link_libraries(configflux_ros2_runtime_lifecycle_mode
  configflux_ros2_runtime_parameter_node_adapter
  configflux_ros2_runtime_control_node_adapter
)
add_dependencies(configflux_ros2_runtime_lifecycle_mode
  ${PROJECT_NAME}__rosidl_generator_cpp
)

add_library(configflux_ros2_runtime_diagnostics_bridge STATIC
  src/runtime_diagnostics_bridge.cc
)
target_include_directories(configflux_ros2_runtime_diagnostics_bridge
  PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../../../cpp/include>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(configflux_ros2_runtime_diagnostics_bridge PUBLIC cxx_std_20)
ament_target_dependencies(configflux_ros2_runtime_diagnostics_bridge rclcpp)

install(
  TARGETS configflux_ros2_runtime_parameter_path
          configflux_ros2_runtime_parameter_adapter
          configflux_ros2_runtime_parameter_node_adapter
          configflux_ros2_runtime_control_adapter
          configflux_ros2_runtime_control_node_adapter
          configflux_ros2_runtime_lifecycle_mode
          configflux_ros2_runtime_diagnostics_bridge
  ARCHIVE DESTINATION lib
  LIBRARY DESTINATION lib
  RUNTIME DESTINATION bin
)

install(
  DIRECTORY include/
  DESTINATION include
)

ament_export_include_directories(include)
ament_export_dependencies(rosidl_default_runtime)

if(BUILD_TESTING)
  find_package(ament_cmake_gtest REQUIRED)

  ament_add_gtest(configflux_ros2_runtime_parameter_node_adapter_test
    test/runtime_parameter_node_adapter_test.cc
    ../../../cpp/src/runtime_session.cc
  )
  if(TARGET configflux_ros2_runtime_parameter_node_adapter_test)
    target_include_directories(configflux_ros2_runtime_parameter_node_adapter_test
      PRIVATE
        ${CMAKE_CURRENT_SOURCE_DIR}/include
        ${CMAKE_CURRENT_SOURCE_DIR}/../../../cpp/include
    )
    target_compile_features(configflux_ros2_runtime_parameter_node_adapter_test
      PRIVATE cxx_std_20
    )
    ament_target_dependencies(configflux_ros2_runtime_parameter_node_adapter_test
      rclcpp
      rcl_interfaces
    )
    target_link_libraries(configflux_ros2_runtime_parameter_node_adapter_test
      configflux_ros2_runtime_parameter_adapter
      configflux_ros2_runtime_parameter_node_adapter
    )
  endif()

  ament_add_gtest(configflux_ros2_runtime_control_node_adapter_test
    test/runtime_control_node_adapter_test.cc
    ../../../cpp/src/runtime_session.cc
  )
  if(TARGET configflux_ros2_runtime_control_node_adapter_test)
    target_include_directories(configflux_ros2_runtime_control_node_adapter_test
      PRIVATE
        ${CMAKE_CURRENT_SOURCE_DIR}/include
        ${CMAKE_CURRENT_SOURCE_DIR}/../../../cpp/include
        ${CMAKE_CURRENT_BINARY_DIR}/rosidl_generator_cpp
    )
    target_compile_features(configflux_ros2_runtime_control_node_adapter_test
      PRIVATE cxx_std_20
    )
    ament_target_dependencies(configflux_ros2_runtime_control_node_adapter_test
      rclcpp
      rclcpp_action
    )
    target_link_libraries(configflux_ros2_runtime_control_node_adapter_test
      configflux_ros2_runtime_control_adapter
      configflux_ros2_runtime_control_node_adapter
      "${cpp_typesupport_target}"
    )
    add_dependencies(configflux_ros2_runtime_control_node_adapter_test
      ${PROJECT_NAME}__rosidl_generator_cpp
    )
  endif()

  ament_add_gtest(configflux_ros2_runtime_lifecycle_mode_test
    test/runtime_lifecycle_mode_test.cc
    ../../../cpp/src/runtime_session.cc
  )
  if(TARGET configflux_ros2_runtime_lifecycle_mode_test)
    target_include_directories(configflux_ros2_runtime_lifecycle_mode_test
      PRIVATE
        ${CMAKE_CURRENT_SOURCE_DIR}/include
        ${CMAKE_CURRENT_SOURCE_DIR}/../../../cpp/include
        ${CMAKE_CURRENT_BINARY_DIR}/rosidl_generator_cpp
    )
    target_compile_features(configflux_ros2_runtime_lifecycle_mode_test
      PRIVATE cxx_std_20
    )
    ament_target_dependencies(configflux_ros2_runtime_lifecycle_mode_test
      rclcpp
      rclcpp_action
      rcl_interfaces
    )
    target_link_libraries(configflux_ros2_runtime_lifecycle_mode_test
      configflux_ros2_runtime_parameter_adapter
      configflux_ros2_runtime_parameter_node_adapter
      configflux_ros2_runtime_control_adapter
      configflux_ros2_runtime_control_node_adapter
      configflux_ros2_runtime_lifecycle_mode
      "${cpp_typesupport_target}"
    )
    add_dependencies(configflux_ros2_runtime_lifecycle_mode_test
      ${PROJECT_NAME}__rosidl_generator_cpp
    )
  endif()
endif()

ament_package()
