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
#ifdef BINDER_STABILITY_SUPPORT
#include <android/binder_stability.h>
#endif  // BINDER_STABILITY_SUPPORT

namespace aidl {
namespace android {
namespace hardware {
namespace secure_element {
class ISecureElementCallbackDelegator;

class ISecureElementCallback : public ::ndk::ICInterface {
public:
  typedef ISecureElementCallbackDelegator DefaultDelegator;
  static const char* descriptor;
  ISecureElementCallback();
  virtual ~ISecureElementCallback();

  static inline const int32_t version = 1;
  static constexpr uint32_t TRANSACTION_onStateChange = FIRST_CALL_TRANSACTION + 0;

  static std::shared_ptr<ISecureElementCallback> fromBinder(const ::ndk::SpAIBinder& binder);
  static binder_status_t writeToParcel(AParcel* parcel, const std::shared_ptr<ISecureElementCallback>& instance);
  static binder_status_t readFromParcel(const AParcel* parcel, std::shared_ptr<ISecureElementCallback>* instance);
  virtual ::ndk::ScopedAStatus onStateChange(bool in_connected, const std::string& in_debugReason) = 0;
  virtual ::ndk::ScopedAStatus getInterfaceVersion(int32_t* _aidl_return) = 0;
};
class ISecureElementCallbackDefault : public ISecureElementCallback {
public:
  ::ndk::ScopedAStatus onStateChange(bool in_connected, const std::string& in_debugReason) override;
  ::ndk::ScopedAStatus getInterfaceVersion(int32_t* _aidl_return) override;
  ::ndk::SpAIBinder asBinder() override;
  bool isRemote() override;
};
}  // namespace secure_element
}  // namespace hardware
}  // namespace android
}  // namespace aidl
