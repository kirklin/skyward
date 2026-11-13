/******************************************************************************
 *
 *  Copyright 2021-2024 NXP
 *
 *  Licensed under the Apache License, Version 2.0 (the "License");
 *  you may not use this file except in compliance with the License.
 *  You may obtain a copy of the License at
 *
 *  http://www.apache.org/licenses/LICENSE-2.0
 *
 *  Unless required by applicable law or agreed to in writing, software
 *  distributed under the License is distributed on an "AS IS" BASIS,
 *  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 *  See the License for the specific language governing permissions and
 *  limitations under the License.
 *
 ******************************************************************************/
#define LOG_TAG "SBAccessController"

#include <android-base/logging.h>
#include <map>
#include <vector>

#include <EseTransportUtils.h>
#include <SBAccessController.h>

#define UPGRADE_OFFSET_SW 3    // upgrade offset from last in response
#define UPGRADE_MASK_BIT 0x02  // Update bit mask in upgrade byte
#define UPGRADE_MASK_VAL 0x02  // Update mask value in upgrade byte

namespace keymint::javacard {

static bool g_AccessAllowed = true;
static std::atomic<uint8_t> g_NumOfCryptoOps = 0;

// These should be in sync with JavacardKeymasterDevice41.cpp
// Allow listed cmds
std::map<uint8_t, uint8_t> allowedCmdIns = {{0x2D /*INS_GET_HMAC_SHARING_PARAM*/, 0},
                                            {0x2A /*INS_COMPUTE_SHARED_HMAC*/, 0},
                                            {0x4D /*INS_GET_ROT_CHALLENGE_CMD*/, 0}};

static void CryptoOpTimerFunc(union sigval arg) {
    (void)arg;  // unused
    LOG(DEBUG) << "CryptoOperation timer expired";
    g_NumOfCryptoOps = 0;
}

static void AccessTimerFunc(union sigval arg) {
    (void)arg;  // unused
    LOG(DEBUG) << "Applet access-block timer expired";
    g_AccessAllowed = true;
}
SBAccessController& SBAccessController::getInstance() {
    static SBAccessController sb_access_cntrl;
    return sb_access_cntrl;
}
void SBAccessController::startTimer(bool isStart, IntervalTimer& t, int timeout,
                                    void (*timerFunc)(union sigval)) {
    t.kill();
    if (isStart) {
        t.set(timeout, this, timerFunc);
    }
}

void SBAccessController::parseResponse(std::vector<uint8_t>& responseApdu) {
    // check if StrongBox Applet update is underway
    if ((responseApdu[responseApdu.size() - UPGRADE_OFFSET_SW] & UPGRADE_MASK_BIT) ==
        UPGRADE_MASK_VAL) {
        mIsUpdateInProgress = true;
        LOG(INFO) << "StrongBox Applet update is in progress";
        g_AccessAllowed = false;  // No access or Limited access
        startTimer(true, mTimer, SB_ACCESS_BLOCK_TIMER, AccessTimerFunc);
    } else {
        mIsUpdateInProgress = false;
        g_AccessAllowed = true;  // Full access
        startTimer(false, mTimer, 0, nullptr);
    }
}
int SBAccessController::getSessionTimeout() {
    if (mIsUpdateInProgress) {
        return (mBootState == BOOTSTATE::SB_EARLY_BOOT_ENDED) ? SMALLEST_SESSION_TIMEOUT
                                                              : UPGRADE_SESSION_TIMEOUT;
    } else {
        return (g_NumOfCryptoOps > 0) ? CRYPTO_OP_SESSION_TIMEOUT : REGULAR_SESSION_TIMEOUT;
    }
}
bool SBAccessController::isSelectAllowed() {
    bool select_allowed = false;
    if (g_AccessAllowed) {
        select_allowed = true;
    } else {
        switch (mBootState) {
            case BOOTSTATE::SB_EARLY_BOOT:
                select_allowed = true;
                break;
            case BOOTSTATE::SB_EARLY_BOOT_ENDED:
                break;
        }
    }
    if(!select_allowed)
        LOG(INFO) << "StrongBox Applet selection is not allowed";

    return select_allowed;
}
void SBAccessController::updateBootState() {
    // set the state to BOOT_ENDED once we have received
    // all allowed commands
    bool allCmdreceived = true;
    for (auto it = allowedCmdIns.begin(); it != allowedCmdIns.end(); it++) {
        if (it->second == 0) {
            allCmdreceived = false;
            break;
        }
    }
    if (allCmdreceived) {
        LOG(INFO) << "All allowlisted cmds received , mark Early boot completed";
        mBootState = BOOTSTATE::SB_EARLY_BOOT_ENDED;
    }
}
void SBAccessController::setCryptoOperationState(uint8_t opState) {
    if (opState == OPERATION_STATE::OP_STARTED) {
        g_NumOfCryptoOps++;
        startTimer(true, mTimerCrypto, CRYPTO_OP_SESSION_TIMEOUT, CryptoOpTimerFunc);
    } else if (opState == OPERATION_STATE::OP_FINISHED) {
        if (g_NumOfCryptoOps > 0) g_NumOfCryptoOps--;
        if (g_NumOfCryptoOps == 0) {
            LOG(INFO) << "All crypto operations finished";
            startTimer(false, mTimerCrypto, 0, nullptr);
        }
    }
    LOG(INFO) << "Number of operations running: " << std::to_string(g_NumOfCryptoOps);
}
bool SBAccessController::isOperationAllowed(uint8_t cmdIns) {
    bool op_allowed = false;
    if (g_AccessAllowed) {
        op_allowed = true;
    } else {
        switch (mBootState) {
            case BOOTSTATE::SB_EARLY_BOOT: {
                auto it = allowedCmdIns.find(cmdIns);
                if (it != allowedCmdIns.end()) {
                    it->second = 1;  // cmd received
                    updateBootState();
                    op_allowed = true;
                }
            } break;
            case BOOTSTATE::SB_EARLY_BOOT_ENDED:
                break;
        }
    }
    if (cmdIns == EARLY_BOOT_ENDED_CMD || cmdIns == INS_SEND_ROT_DATA_CMD) {
        // allowed as these may be received during early boot
        op_allowed = true;
    }
    return op_allowed;
}
}  // namespace keymint::javacard
