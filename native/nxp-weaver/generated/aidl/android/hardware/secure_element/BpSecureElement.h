/*
 * This file is auto-generated.  DO NOT MODIFY.
 * Using: cmd not shown due to `--omit_invocation`
 *
 * DO NOT CHECK THIS FILE INTO A CODE TREE (e.g. git, etc..).
 * ALWAYS GENERATE THIS FILE FROM UPDATED AIDL COMPILER
 * AS A BUILD INTERMEDIATE ONLY. THIS IS NOT SOURCE CODE.
 */
#pragma once

#include "aidl/android/hardware/secure_element/ISecureElement.h"

#include <android/binder_ibinder.h>

namespace aidl {
namespace android {
namespace hardware {
namespace secure_element {
class BpSecureElement : public ::ndk::BpCInterface<ISecureElement> {
public:
  explicit BpSecureElement(const ::ndk::SpAIBinder& binder);
  virtual ~BpSecureElement();

  ::ndk::ScopedAStatus closeChannel(int8_t in_channelNumber) override;
  ::ndk::ScopedAStatus getAtr(std::vector<uint8_t>* _aidl_return) override;
  ::ndk::ScopedAStatus init(const std::shared_ptr<::aidl::android::hardware::secure_element::ISecureElementCallback>& in_clientCallback) override;
  ::ndk::ScopedAStatus isCardPresent(bool* _aidl_return) override;
  ::ndk::ScopedAStatus openBasicChannel(const std::vector<uint8_t>& in_aid, int8_t in_p2, std::vector<uint8_t>* _aidl_return) override;
  ::ndk::ScopedAStatus openLogicalChannel(const std::vector<uint8_t>& in_aid, int8_t in_p2, ::aidl::android::hardware::secure_element::LogicalChannelResponse* _aidl_return) override;
  ::ndk::ScopedAStatus reset() override;
  ::ndk::ScopedAStatus transmit(const std::vector<uint8_t>& in_data, std::vector<uint8_t>* _aidl_return) override;
  ::ndk::ScopedAStatus getInterfaceVersion(int32_t* _aidl_return) override;
  int32_t _aidl_cached_version = -1;
};
}  // namespace secure_element
}  // namespace hardware
}  // namespace android
}  // namespace aidl
