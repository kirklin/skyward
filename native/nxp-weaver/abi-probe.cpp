#include <HalToHalTransport.h>
#include <weaver-impl.h>
#include <cstdint>
using namespace keymint::javacard;
static_assert(sizeof(SlotInfo) == 12);
static_assert(sizeof(ReadRespInfo) == 32);
static_assert(sizeof(WeaverImpl) == 24);
static_assert(sizeof(AppletConnection) == 120); // This transport is rebuilt privately, never shared with vendor C++ classes.
static_assert(sizeof(HalToHalTransport) == 144);
extern "C" int layout_probe() { return sizeof(HalToHalTransport); }
