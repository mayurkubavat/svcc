#pragma once
#include <cstdint>

// svcc:c2rtl
uint16_t gray_enc16_golden(uint16_t bin_in) {
  uint16_t g = bin_in ^ (bin_in >> 1U);
  return g;
}
