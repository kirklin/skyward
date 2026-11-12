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
#include <cassert>

#ifndef __BIONIC__
#ifndef __assert2
#define __assert2(a,b,c,d) ((void)0)
#endif
#endif

namespace aidl {
namespace android {
namespace hardware {
namespace secure_element {
class BnSecureElementCallback : public ::ndk::BnCInterface<ISecureElementCallback> {
public:
  BnSecureElementCallback();
  virtual ~BnSecureElementCallback();
  ::ndk::ScopedAStatus getInterfaceVersion(int32_t* _aidl_return) final;
protected:
  ::ndk::SpAIBinder createBinder() override;
private:
};
class ISecureElementCallbackDelegator : public BnSecureElementCallback {
public:
  explicit ISecureElementCallbackDelegator(const std::shared_ptr<ISecureElementCallback> &impl) : _impl(impl) {
     int32_t _impl_ver = 0;
     if (!impl->getInterfaceVersion(&_impl_ver).isOk()) {;
        __assert2(__FILE__, __LINE__, __PRETTY_FUNCTION__, "Delegator failed to get version of the implementation.");
     }
     if (_impl_ver != ISecureElementCallback::version) {
        __assert2(__FILE__, __LINE__, __PRETTY_FUNCTION__, "Mismatched versions of delegator and implementation is not allowed.");
     }
  }

  ::ndk::ScopedAStatus onStateChange(bool in_connected, const std::string& in_debugReason) override {
    return _impl->onStateChange(in_connected, in_debugReason);
  }
protected:
private:
  std::shared_ptr<ISecureElementCallback> _impl;
};

}  // namespace secure_element
}  // namespace hardware
}  // namespace android
}  // namespace aidl
