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
#ifndef __APPLETCONNECTION_H__
#define __APPLETCONNECTION_H__

#include <aidl/android/hardware/secure_element/BnSecureElementCallback.h>
#include <aidl/android/hardware/secure_element/ISecureElement.h>
#include <vector>

#include <SBAccessController.h>

namespace keymint::javacard {
class SecureElementCallback;
using aidl::android::hardware::secure_element::ISecureElement;

struct AppletConnection {
public:
  AppletConnection(const std::vector<uint8_t>& aid);

  /**
   * Connects to the secure element HAL service. Returns true if successful, false otherwise.
   */
  bool connectToSEService();

  /**
   * Select the applet on the secure element. SELECT command response is returned in resp vector
   */
  bool openChannelToApplet(std::vector<uint8_t>& resp);

  /**
   * If open, closes the open channel to the applet. Returns an error if channel was not
   * open or the SE HAL service returned an error.
   */
  bool close();

  /**
   * Sends the data to the secure element and also receives back the data.
   * This is a blocking call.
   */
  bool transmit(std::vector<uint8_t>& CommandApdu, std::vector<uint8_t>& output);

  /**
   * Checks if a channel to the applet is open.
   */
  bool isChannelOpen();

  /**
   * Checks if service is connected to eSE HAL.
   */
  bool isServiceConnected();
  /**
   * Get session timeout value based on select response normal/update session
   */
  int getSessionTimeout();

 private:
  /**
   * Select applet with given P2 parameter
   */
  bool selectApplet(std::vector<uint8_t>& resp, uint8_t p2);

  std::mutex channel_mutex_;  // exclusive access to isChannelopen()/close()

  std::shared_ptr<ISecureElement> mSecureElement;
  std::shared_ptr<SecureElementCallback> mSecureElementCallback;
  ::ndk::ScopedAIBinder_DeathRecipient mDeathRecipient;
  static void BinderDiedCallback(void* cookie);
  std::vector<uint8_t> kAppletAID;
  int8_t mOpenChannel = -1;
  SBAccessController& mSBAccessController;
};

}  // namespace keymint::javacard
#endif  // __APPLETCONNECTION_H__
