/*
**
** Copyright 2018, The Android Open Source Project
**
** Licensed under the Apache License, Version 2.0 (the "License");
** you may not use this file except in compliance with the License.
** You may obtain a copy of the License at
**
**     http://www.apache.org/licenses/LICENSE-2.0
**
** Unless required by applicable law or agreed to in writing, software
** distributed under the License is distributed on an "AS IS" BASIS,
** WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
** See the License for the specific language governing permissions and
** limitations under the License.
*/
/******************************************************************************
 **
 ** The original Work has been changed by NXP.
 **
 ** Licensed under the Apache License, Version 2.0 (the "License");
 ** you may not use this file except in compliance with the License.
 ** You may obtain a copy of the License at
 **
 ** http://www.apache.org/licenses/LICENSE-2.0
 **
 ** Unless required by applicable law or agreed to in writing, software
 ** distributed under the License is distributed on an "AS IS" BASIS,
 ** WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 ** See the License for the specific language governing permissions and
 ** limitations under the License.
 **
 ** Copyright 2020-2021,2024 NXP
 **
 *********************************************************************************/
#define LOG_TAG "AppletConnection"

#include <android-base/logging.h>
#include <android-base/stringprintf.h>
#include <android/binder_manager.h>
#include <signal.h>
#include <iomanip>
#include <mutex>
#include <string>
#include <vector>

#include <AppletConnection.h>
#include <EseTransportUtils.h>
#include <SignalHandler.h>

using aidl::android::hardware::secure_element::BnSecureElementCallback;
using aidl::android::hardware::secure_element::ISecureElement;
using aidl::android::hardware::secure_element::LogicalChannelResponse;
using android::base::StringPrintf;
using ndk::ScopedAStatus;
using ndk::SharedRefBase;
using ndk::SpAIBinder;

namespace keymint::javacard {

static bool isStrongBox = false; // true when linked with StrongBox HAL process
const std::vector<uint8_t> kStrongBoxAppletAID = {0xA0, 0x00, 0x00, 0x00, 0x62};
constexpr const char eseHalServiceName[] = "android.hardware.secure_element.ISecureElement/eSE1";

class SecureElementCallback : public BnSecureElementCallback {
  public:
    ScopedAStatus onStateChange(bool state, const std::string& in_debugReason) override {
        LOGD_OMAPI("connected =" << (state ? "true " : "false ") << "reason: " << in_debugReason);
        mConnState = state;
        return ScopedAStatus::ok();
    };
    bool isClientConnected() { return mConnState; }

  private:
    bool mConnState = false;
};

void AppletConnection::BinderDiedCallback(void* cookie) {
    LOG(ERROR) << "Received binder death ntf. SE HAL Service died";
    auto thiz = static_cast<AppletConnection*>(cookie);
    thiz->mSecureElementCallback->onStateChange(false, "SE HAL died");
    thiz->mSecureElement = nullptr;
}

AppletConnection::AppletConnection(const std::vector<uint8_t>& aid)
    : kAppletAID(aid), mSBAccessController(SBAccessController::getInstance()) {
    if (kAppletAID == kStrongBoxAppletAID) {
        isStrongBox = true;
    }
    mDeathRecipient =
        ::ndk::ScopedAIBinder_DeathRecipient(AIBinder_DeathRecipient_new(BinderDiedCallback));
}

bool AppletConnection::connectToSEService() {
    if (!SignalHandler::getInstance()->isHandlerRegistered()) {
        LOG(DEBUG) << "register signal handler";
        SignalHandler::getInstance()->installHandler(this);
    }
    if (mSecureElement != nullptr && mSecureElementCallback->isClientConnected()) {
        LOG(INFO) <<"Already connected";
        return true;
    }
    bool connected = false;
    SpAIBinder binder = SpAIBinder(AServiceManager_waitForService(eseHalServiceName));
    mSecureElement = ISecureElement::fromBinder(binder);
    if (mSecureElement == nullptr) {
        LOG(ERROR) << "Failed to connect to Secure element service";
    } else {
        mSecureElementCallback = SharedRefBase::make<SecureElementCallback>();
        auto status = mSecureElement->init(mSecureElementCallback);
        connected = status.isOk() && mSecureElementCallback->isClientConnected();
        if (!connected) {
            LOG(ERROR) << "Failed to initialize SE HAL service";
        }
    }
    return connected;
}

// AIDL Hal returns empty response for failure case
// so prepare response based on service specific errorcode
void prepareServiceSpecificErrorRepsponse(std::vector<uint8_t>& resp, int32_t errorCode) {
    resp.clear();
    switch (errorCode) {
        case ISecureElement::NO_SUCH_ELEMENT_ERROR:
            resp.push_back(0x6A);
            resp.push_back(0x82);
            break;
        case ISecureElement::CHANNEL_NOT_AVAILABLE:
            resp.push_back(0x6A);
            resp.push_back(0x81);
            break;
        case ISecureElement::UNSUPPORTED_OPERATION:
            resp.push_back(0x6A);
            resp.push_back(0x86);
            break;
        case ISecureElement::IOERROR:
            resp.push_back(0x64);
            resp.push_back(0xFF);
            break;
        default:
            resp.push_back(0xFF);
            resp.push_back(0xFF);
    }
}
bool AppletConnection::selectApplet(std::vector<uint8_t>& resp, uint8_t p2) {
  bool stat = false;
  resp.clear();
  LogicalChannelResponse logical_channel_response;
  auto status = mSecureElement->openLogicalChannel(kAppletAID, p2, &logical_channel_response);
  if (status.isOk()) {
      mOpenChannel = logical_channel_response.channelNumber;
      resp = logical_channel_response.selectResponse;
      stat = true;
  } else {
      mOpenChannel = -1;
      resp = logical_channel_response.selectResponse;
      LOG(ERROR) << "openLogicalChannel: Failed ";
      // AIDL Hal returns empty response for failure case
      // so prepare response based on service specific errorcode
      prepareServiceSpecificErrorRepsponse(resp, status.getServiceSpecificError());
  }
  return stat;
}
void prepareErrorRepsponse(std::vector<uint8_t>& resp){
        resp.clear();
        resp.push_back(0xFF);
        resp.push_back(0xFF);
}
bool AppletConnection::openChannelToApplet(std::vector<uint8_t>& resp) {
  bool ret = false;
  uint8_t retry = 0;
  if (isChannelOpen()) {
    LOG(INFO) << "channel Already opened";
    return true;
  }
  if (isStrongBox) {
      if (!mSBAccessController.isSelectAllowed()) {
          prepareErrorRepsponse(resp);
          return false;
      }
      do {
          if (selectApplet(resp, SELECT_P2_VALUE_0) || selectApplet(resp, SELECT_P2_VALUE_2)) {
              ret = true;
              break;
          }
          LOG(INFO) << " openChannelToApplet retry after 2 secs";
          usleep(2 * ONE_SEC);
      } while (++retry < MAX_RETRY_COUNT);
  } else {
      ret = selectApplet(resp, 0x0);
  }
  return ret;
}

bool AppletConnection::transmit(std::vector<uint8_t>& CommandApdu , std::vector<uint8_t>& output){
    std::vector<uint8_t> cmd = CommandApdu;
    cmd[0] |= mOpenChannel ;
    LOGD_OMAPI("Channel number: " << static_cast<int>(mOpenChannel));

    if (mSecureElement == nullptr) return false;
    if (isStrongBox) {
        if (!mSBAccessController.isOperationAllowed(CommandApdu[APDU_INS_OFFSET])) {
            std::vector<uint8_t> ins;
            ins.push_back(CommandApdu[APDU_INS_OFFSET]);
            LOG(ERROR) << "command Ins:" << ins << " not allowed";
            prepareErrorRepsponse(output);
            return false;
        }
    }
    // block any fatal signal delivery
    SignalHandler::getInstance()->blockSignals();
    std::vector<uint8_t> response;
    mSecureElement->transmit(cmd, &response);
    output = response;
    // un-block signal delivery
    SignalHandler::getInstance()->unblockSignals();
    return true;
}

int AppletConnection::getSessionTimeout() {
    return mSBAccessController.getSessionTimeout();
}

bool AppletConnection::close() {
    std::lock_guard<std::mutex> lock(channel_mutex_);
    if (mSecureElement == nullptr) {
        LOG(ERROR) << "Channel couldn't be closed mSEClient handle is null";
        return false;
    }
    if(mOpenChannel < 0){
       LOG(INFO) << "Channel is already closed";
       return true;
    }
    auto status = mSecureElement->closeChannel(mOpenChannel);
    if (!status.isOk()) {
        /*
         * reason could be SE reset or HAL deinit triggered from other client
         * which anyway closes all the opened channels
         */
        LOG(ERROR) << "closeChannel failed";
        mOpenChannel = -1;
        return true;
    }
    LOG(INFO) << "Channel closed";
    mOpenChannel = -1;
    return true;
}

bool AppletConnection::isServiceConnected() {
    std::lock_guard<std::mutex> lock(channel_mutex_);
    if (mSecureElement == nullptr || !mSecureElementCallback->isClientConnected()) {
        return false;
    }
    return true;
}

bool AppletConnection::isChannelOpen() {
    std::lock_guard<std::mutex> lock(channel_mutex_);
    return mOpenChannel >= 0;
}
}  // namespace keymint::javacard
