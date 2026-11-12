#pragma once
#include <cstdint>
#include <vector>
#include <memory>
#include <deque>
#include <cassert>
#include <utility>
namespace fake {
struct Reply { bool ok; std::vector<uint8_t> bytes; bool writes_response=true; };
inline std::deque<Reply> replies;
inline std::vector<std::vector<uint8_t>> sends;
inline std::vector<std::vector<uint8_t>> aid_changes;
}
namespace se_transport {
class TransportFactory {
public:
  TransportFactory(bool, const std::vector<uint8_t>&) {}
  bool openConnection() { return true; }
  bool closeConnection() { return true; }
  bool setAppletAid(const std::vector<uint8_t>& aid) {
    fake::aid_changes.push_back(aid); return true;
  }
  bool sendData(const uint8_t* data, size_t size, std::vector<uint8_t>& response) {
    fake::sends.emplace_back(data,data+size);
    assert(!fake::replies.empty());
    auto reply=std::move(fake::replies.front());fake::replies.pop_front();
    if (reply.writes_response) response=std::move(reply.bytes);
    return reply.ok;
  }
};
}
