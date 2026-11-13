// This test links the production patched transport implementation to a fake.
// It has no Android/Binder/secure-element dependency and performs no real APDU.
#include <TransportFactory.h>
#include <weaver_transport-impl.h>
#include <weaver_parser-impl.h>
#include <iostream>

static const std::vector<std::vector<uint8_t>> aids={{1},{2},{3}};
static const std::vector<uint8_t> request={0x00,0x00,0x00,0x00};
static void scenario(const char* label, std::deque<fake::Reply> replies,
                     bool expected, size_t count, std::vector<std::vector<uint8_t>> changes={}) {
  fake::replies=std::move(replies);fake::sends.clear();fake::aid_changes.clear();
  auto transport=WeaverTransportImpl::getInstance();transport->Init(aids);
  std::vector<uint8_t> response={0x6A,0x82}; // A previous call must not cause fallback.
  assert(transport->Send(request,response)==expected);
  assert(fake::sends.size()==count);
  assert(fake::replies.empty());
  assert(fake::aid_changes==changes);
  for(const auto& sent:fake::sends) assert(sent==request);
  std::cout << label << " PASS sends=" << count << '\n';
}
int main() {
  scenario("success",{{true,{0x90,0x00}}},true,1);
  scenario("wrong-key-status",{{true,{0x7F,0x90,0x00}}},true,1);
  scenario("throttle-response",{{true,{0x76,0x90,0x00}}},true,1);
  scenario("ambiguous-failure",{{false,{}}},false,1);
  scenario("failure-without-response",{{false,{},false}},false,1);
  scenario("short-failure-response",{{false,{0x6A}}},false,1);
  scenario("non-select-failure",{{false,{0x64,0xFF}}},false,1);
  scenario("three-AID-fallback",{{false,{0x6A,0x82}},{false,{0x6A,0x82}},{true,{0x90,0x00}}},true,3,{{2},{3}});
  scenario("all-AIDs-missing",{{false,{0x6A,0x82}},{false,{0x6A,0x82}},{false,{0x6A,0x82}}},false,3,{{2},{3},{1}});
  scenario("stop-after-ambiguous-alternate",{{false,{0x6A,0x82}},{false,{}}},false,2,{{2},{1}});
  scenario("alternate-stale-selection-error",{{false,{0x6A,0x82}},{false,{},false}},false,2,{{2},{1}});
  scenario("alternate-wrong-key-status",{{false,{0x6A,0x82}},{true,{0x7F,0x90,0x00}}},true,2,{{2}});
  auto parser=WeaverParserImpl::getInstance();
  ReadRespInfo read;
  assert(parser->ParseReadInfo({0x7F,0x90,0x00},read)==WEAVER_STATUS_INCORRECT_KEY);
  assert(parser->ParseReadInfo({0x76,0x90,0x00},read)==WEAVER_STATUS_THROTTLE);
  std::vector<std::vector<uint8_t>> actual_aids;
  assert(parser->getAppletId(actual_aids));
  const std::vector<std::vector<uint8_t>> live_aids={
    {0xA0,0x00,0x00,0x03,0x96,0x10,0x10},
    {0xA0,0x00,0x00,0x03,0x96,0x54,0x53,0x00,0x00,0x00,0x01,0x00,0x23},
    {0xA0,0x00,0x00,0x03,0x96,0x10,0x10,0x00,0x00,0x00,0x01,0x00,0x23,0x00,0x00,0x00}};
  assert(actual_aids==live_aids);
  std::cout << "parser-statuses-and-exact-live-AIDs PASS\n";
}
