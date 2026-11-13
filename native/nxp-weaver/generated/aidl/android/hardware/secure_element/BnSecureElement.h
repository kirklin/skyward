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
class BnSecureElement : public ::ndk::BnCInterface<ISecureElement> {
public:
  BnSecureElement();
  virtual ~BnSecureElement();
  ::ndk::ScopedAStatus getInterfaceVersion(int32_t* _aidl_return) final;
protected:
  ::ndk::SpAIBinder createBinder() override;
private:
};
class ISecureElementDelegator : public BnSecureElement {
public:
  explicit ISecureElementDelegator(const std::shared_ptr<ISecureElement> &impl) : _impl(impl) {
     int32_t _impl_ver = 0;
     if (!impl->getInterfaceVersion(&_impl_ver).isOk()) {;
        __assert2(__FILE__, __LINE__, __PRETTY_FUNCTION__, "Delegator failed to get version of the implementation.");
     }
     if (_impl_ver != ISecureElement::version) {
        __assert2(__FILE__, __LINE__, __PRETTY_FUNCTION__, "Mismatched versions of delegator and implementation is not allowed.");
     }
  }

  ::ndk::ScopedAStatus closeChannel(int8_t in_channelNumber) override {
    return _impl->closeChannel(in_channelNumber);
  }
  ::ndk::ScopedAStatus getAtr(std::vector<uint8_t>* _aidl_return) override {
    return _impl->getAtr(_aidl_return);
  }
  ::ndk::ScopedAStatus init(const std::shared_ptr<::aidl::android::hardware::secure_element::ISecureElementCallback>& in_clientCallback) override {
    return _impl->init(in_clientCallback);
  }
  ::ndk::ScopedAStatus isCardPresent(bool* _aidl_return) override {
    return _impl->isCardPresent(_aidl_return);
  }
  ::ndk::ScopedAStatus openBasicChannel(const std::vector<uint8_t>& in_aid, int8_t in_p2, std::vector<uint8_t>* _aidl_return) override {
    return _impl->openBasicChannel(in_aid, in_p2, _aidl_return);
  }
  ::ndk::ScopedAStatus openLogicalChannel(const std::vector<uint8_t>& in_aid, int8_t in_p2, ::aidl::android::hardware::secure_element::LogicalChannelResponse* _aidl_return) override {
    return _impl->openLogicalChannel(in_aid, in_p2, _aidl_return);
  }
  ::ndk::ScopedAStatus reset() override {
    return _impl->reset();
  }
  ::ndk::ScopedAStatus transmit(const std::vector<uint8_t>& in_data, std::vector<uint8_t>* _aidl_return) override {
    return _impl->transmit(in_data, _aidl_return);
  }
protected:
private:
  std::shared_ptr<ISecureElement> _impl;
};

}  // namespace secure_element
}  // namespace hardware
}  // namespace android
}  // namespace aidl
