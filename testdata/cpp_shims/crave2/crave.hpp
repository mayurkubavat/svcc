#pragma once
#include <cstdint>
#include <initializer_list>
#include <string>

namespace crave {
struct expr {
  constexpr expr() = default;
  template <typename T>
  constexpr expr(const T&) {}
};

#define UVIR_CRAVE_BINOP(OP) \
  inline constexpr expr operator OP(expr, expr) { return {}; } \
  template <typename R> inline constexpr expr operator OP(expr, const R&) { return {}; } \
  template <typename L> inline constexpr expr operator OP(const L&, expr) { return {}; }

UVIR_CRAVE_BINOP(==)
UVIR_CRAVE_BINOP(!=)
UVIR_CRAVE_BINOP(<)
UVIR_CRAVE_BINOP(<=)
UVIR_CRAVE_BINOP(>)
UVIR_CRAVE_BINOP(>=)
UVIR_CRAVE_BINOP(+)
UVIR_CRAVE_BINOP(-)
UVIR_CRAVE_BINOP(*)
UVIR_CRAVE_BINOP(%)
UVIR_CRAVE_BINOP(&)
UVIR_CRAVE_BINOP(|)
UVIR_CRAVE_BINOP(^)
UVIR_CRAVE_BINOP(<<)
UVIR_CRAVE_BINOP(>>)
UVIR_CRAVE_BINOP(&&)
UVIR_CRAVE_BINOP(||)
#undef UVIR_CRAVE_BINOP

inline constexpr expr operator!(expr) { return {}; }
inline constexpr expr operator~(expr) { return {}; }
inline constexpr expr operator-(expr) { return {}; }

inline constexpr expr if_then(expr, expr) { return {}; }

template <typename T>
struct distribution {
  distribution& add(uint64_t, uint32_t) { return *this; }
  distribution& add_range(uint64_t, uint64_t, uint32_t) { return *this; }
  distribution& add_weighted_range(uint64_t, uint64_t, uint32_t) { return *this; }
};

template <typename T>
inline constexpr expr dist(expr, const distribution<T>&) { return {}; }

template <typename T>
class crv_variable {
  T val_{};
public:
  crv_variable() = default;
  explicit crv_variable(const char*) {}
  crv_variable& operator=(const T& v) { val_ = v; return *this; }
  operator T() const { return val_; }
  expr operator()() const { return {}; }
};

struct crv_constraint {
  template <typename Owner, typename... Exprs>
  crv_constraint(Owner*, const char*, Exprs&&...) {}
};

struct crv_soft_constraint {
  template <typename Owner, typename... Exprs>
  crv_soft_constraint(Owner*, const char*, Exprs&&...) {}
};
} // namespace crave
