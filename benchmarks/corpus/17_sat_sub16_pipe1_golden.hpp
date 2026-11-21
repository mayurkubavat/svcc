#pragma once
#include <cstdint>

// svcc:c2rtl
uint16_t sat_sub16_golden(uint16_t x, uint16_t y) {
  uint16_t d = 0U;
  if (x >= y) {
    d = x - y;
  }
  return d;
}
