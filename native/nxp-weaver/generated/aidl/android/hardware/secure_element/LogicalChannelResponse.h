/*
 * This file is auto-generated.  DO NOT MODIFY.
 * Using: cmd not shown due to `--omit_invocation`
 *
 * DO NOT CHECK THIS FILE INTO A CODE TREE (e.g. git, etc..).
 * ALWAYS GENERATE THIS FILE FROM UPDATED AIDL COMPILER
 * AS A BUILD INTERMEDIATE ONLY. THIS IS NOT SOURCE CODE.
 */
#pragma once

#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <vector>
#include <android/binder_interface_utils.h>
#include <android/binder_parcelable_utils.h>
#include <android/binder_to_string.h>
#ifdef BINDER_STABILITY_SUPPORT
#include <android/binder_stability.h>
#endif  // BINDER_STABILITY_SUPPORT

namespace aidl {
namespace android {
namespace hardware {
namespace secure_element {
class LogicalChannelResponse {
public:
  typedef std::false_type fixed_size;
  static const char* descriptor;

  int8_t channelNumber = 0;
  std::vector<uint8_t> selectResponse;

  binder_status_t readFromParcel(const AParcel* parcel);
  binder_status_t writeToParcel(AParcel* parcel) const;

  inline bool operator==(const LogicalChannelResponse& _rhs) const {
    return std::tie(channelNumber, selectResponse) == std::tie(_rhs.channelNumber, _rhs.selectResponse);
  }
  inline bool operator<(const LogicalChannelResponse& _rhs) const {
    return std::tie(channelNumber, selectResponse) < std::tie(_rhs.channelNumber, _rhs.selectResponse);
  }
  inline bool operator!=(const LogicalChannelResponse& _rhs) const {
    return !(*this == _rhs);
  }
  inline bool operator>(const LogicalChannelResponse& _rhs) const {
    return _rhs < *this;
  }
  inline bool operator>=(const LogicalChannelResponse& _rhs) const {
    return !(*this < _rhs);
  }
  inline bool operator<=(const LogicalChannelResponse& _rhs) const {
    return !(_rhs < *this);
  }

  static const ::ndk::parcelable_stability_t _aidl_stability = ::ndk::STABILITY_VINTF;
  inline std::string toString() const {
    std::ostringstream _aidl_os;
    _aidl_os << "LogicalChannelResponse{";
    _aidl_os << "channelNumber: " << ::android::internal::ToString(channelNumber);
    _aidl_os << ", selectResponse: " << ::android::internal::ToString(selectResponse);
    _aidl_os << "}";
    return _aidl_os.str();
  }
};
}  // namespace secure_element
}  // namespace hardware
}  // namespace android
}  // namespace aidl
