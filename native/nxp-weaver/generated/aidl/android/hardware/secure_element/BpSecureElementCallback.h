/*
 * This file is auto-generated.  DO NOT MODIFY.
 * Using: cmd not shown due to `--omit_invocation`
 *
 * DO NOT CHECK THIS FILE INTO A CODE TREE (e.g. git, etc..).
 * ALWAYS GENERATE THIS FILE FROM UPDATED AIDL COMPILER
 * AS A BUILD INTERMEDIATE ONLY. THIS IS NOT SOURCE CODE.
 */
#pragma once

#include "aidl/android/hardware/secure_element/ISecureElementCallback.h"

#include <android/binder_ibinder.h>

namespace aidl {
namespace android {
namespace hardware {
namespace secure_element {
class BpSecureElementCallback : public ::ndk::BpCInterface<ISecureElementCallback> {
public:
  explicit BpSecureElementCallback(const ::ndk::SpAIBinder& binder);
  virtual ~BpSecureElementCallback();

  ::ndk::ScopedAStatus onStateChange(bool in_connected, const std::string& in_debugReason) override;
  ::ndk::ScopedAStatus getInterfaceVersion(int32_t* _aidl_return) override;
  int32_t _aidl_cached_version = -1;
};
}  // namespace secure_element
}  // namespace hardware
}  // namespace android
}  // namespace aidl
