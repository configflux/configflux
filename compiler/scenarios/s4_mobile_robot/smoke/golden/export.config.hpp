#pragma once

namespace configflux::buildcfg {
inline constexpr const char* kBatteryManagerBatteryProfile = "high_density";
inline constexpr const char* kDriveStackDriveDriver = "mecanum_drive_driver";
inline constexpr const char* kDriveStackDriveProfile = "mecanum";
inline constexpr const char* kLocalizationStackLocalizationDriver = "lidar_localization_driver";
inline constexpr const char* kPayloadStackPayloadDriver = "payload_heavy_driver";
}  // namespace configflux::buildcfg
