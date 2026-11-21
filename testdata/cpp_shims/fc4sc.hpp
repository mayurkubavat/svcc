#pragma once
#include <cstdint>
#include <initializer_list>
#include <string>

namespace fc4sc {
struct range_t {
  uint64_t lo;
  uint64_t hi;
};

inline constexpr range_t interval(uint64_t lo, uint64_t hi) { return {lo, hi}; }

template <typename T>
struct bin {
  template <typename... Args>
  explicit bin(const char*, Args&&...) {}
};

template <typename T>
struct ignore_bin : public bin<T> {
  template <typename... Args>
  explicit ignore_bin(const char* nm, Args&&... args) : bin<T>(nm, args...) {}
};

template <typename T>
struct coverpoint {
  coverpoint() = default;
  coverpoint(std::initializer_list<bin<T>>) {}
};

class covergroup {
public:
  covergroup() = default;
  virtual ~covergroup() = default;
  void sample() {}
};

template <typename... Ts>
struct cross {
  template <typename Owner, typename... Cps>
  cross(Owner*, const char*, Cps*...) {}
};
} // namespace fc4sc

#define CG_CONS(T) T()
#define COVERPOINT(TY, NAME, EXPR) ::fc4sc::coverpoint<TY> NAME = ::fc4sc::coverpoint<TY>
