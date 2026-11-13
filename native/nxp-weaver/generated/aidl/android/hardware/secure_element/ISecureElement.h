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
#include <aidl/android/hardware/secure_element/ISecureElementCallback.h>
#include <aidl/android/hardware/secure_element/LogicalChannelResponse.h>
#ifdef BINDER_STABILITY_SUPPORT
#include <android/binder_stability.h>
#endif  // BINDER_STABILITY_SUPPORT

namespace aidl::android::hardware::secure_element {
class ISecureElementCallback;
class LogicalChannelResponse;
}  // namespace aidl::android::hardware::secure_element
namespace aidl {
namespace android {
namespace hardware {
namespace secure_element {
class ISecureElementDelegator;

class ISecureElement : public ::ndk::ICInterface {
public:
  typedef ISecureElementDelegator DefaultDelegator;
  static const char* descriptor;
  ISecureElement();
  virtual ~ISecureElement();

  enum : int32_t { FAILED = 1 };
  enum : int32_t { CHANNEL_NOT_AVAILABLE = 2 };
  enum : int32_t { NO_SUCH_ELEMENT_ERROR = 3 };
  enum : int32_t { UNSUPPORTED_OPERATION = 4 };
  enum : int32_t { IOERROR = 5 };
  static inline const int32_t version = 1;
  static constexpr uint32_t TRANSACTION_closeChannel = FIRST_CALL_TRANSACTION + 0;
  static constexpr uint32_t TRANSACTION_getAtr = FIRST_CALL_TRANSACTION + 1;
  static constexpr uint32_t TRANSACTION_init = FIRST_CALL_TRANSACTION + 2;
  static constexpr uint32_t TRANSACTION_isCardPresent = FIRST_CALL_TRANSACTION + 3;
  static constexpr uint32_t TRANSACTION_openBasicChannel = FIRST_CALL_TRANSACTION + 4;
  static constexpr uint32_t TRANSACTION_openLogicalChannel = FIRST_CALL_TRANSACTION + 5;
  static constexpr uint32_t TRANSACTION_reset = FIRST_CALL_TRANSACTION + 6;
  static constexpr uint32_t TRANSACTION_transmit = FIRST_CALL_TRANSACTION + 7;

  static std::shared_ptr<ISecureElement> fromBinder(const ::ndk::SpAIBinder& binder);
  static binder_status_t writeToParcel(AParcel* parcel, const std::shared_ptr<ISecureElement>& instance);
  static binder_status_t readFromParcel(const AParcel* parcel, std::shared_ptr<ISecureElement>* instance);
  virtual ::ndk::ScopedAStatus closeChannel(int8_t in_channelNumber) = 0;
  virtual ::ndk::ScopedAStatus getAtr(std::vector<uint8_t>* _aidl_return) = 0;
  virtual ::ndk::ScopedAStatus init(const std::shared_ptr<::aidl::android::hardware::secure_element::ISecureElementCallback>& in_clientCallback) = 0;
  virtual ::ndk::ScopedAStatus isCardPresent(bool* _aidl_return) = 0;
  virtual ::ndk::ScopedAStatus openBasicChannel(const std::vector<uint8_t>& in_aid, int8_t in_p2, std::vector<uint8_t>* _aidl_return) = 0;
  virtual ::ndk::ScopedAStatus openLogicalChannel(const std::vector<uint8_t>& in_aid, int8_t in_p2, ::aidl::android::hardware::secure_element::LogicalChannelResponse* _aidl_return) = 0;
  virtual ::ndk::ScopedAStatus reset() = 0;
  virtual ::ndk::ScopedAStatus transmit(const std::vector<uint8_t>& in_data, std::vector<uint8_t>* _aidl_return) = 0;
  virtual ::ndk::ScopedAStatus getInterfaceVersion(int32_t* _aidl_return) = 0;
};
class ISecureElementDefault : public ISecureElement {
public:
  ::ndk::ScopedAStatus closeChannel(int8_t in_channelNumber) override;
  ::ndk::ScopedAStatus getAtr(std::vector<uint8_t>* _aidl_return) override;
  ::ndk::ScopedAStatus init(const std::shared_ptr<::aidl::android::hardware::secure_element::ISecureElementCallback>& in_clientCallback) override;
  ::ndk::ScopedAStatus isCardPresent(bool* _aidl_return) override;
  ::ndk::ScopedAStatus openBasicChannel(const std::vector<uint8_t>& in_aid, int8_t in_p2, std::vector<uint8_t>* _aidl_return) override;
  ::ndk::ScopedAStatus openLogicalChannel(const std::vector<uint8_t>& in_aid, int8_t in_p2, ::aidl::android::hardware::secure_element::LogicalChannelResponse* _aidl_return) override;
  ::ndk::ScopedAStatus reset() override;
  ::ndk::ScopedAStatus transmit(const std::vector<uint8_t>& in_data, std::vector<uint8_t>* _aidl_return) override;
  ::ndk::ScopedAStatus getInterfaceVersion(int32_t* _aidl_return) override;
  ::ndk::SpAIBinder asBinder() override;
  bool isRemote() override;
};
}  // namespace secure_element
}  // namespace hardware
}  // namespace android
}  // namespace aidl
