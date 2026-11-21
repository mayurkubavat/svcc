#pragma once
#include <cstdint>

// svcc:c2rtl
uint32_t minmax32_golden(bool pick_max, uint32_t a, uint32_t b) {
  uint32_t out = a;
  if (pick_max) {
    if (b > a) {
      out = b;
    }
  } else {
    if (b < a) {
      out = b;
    }
  }
  return out;
}
