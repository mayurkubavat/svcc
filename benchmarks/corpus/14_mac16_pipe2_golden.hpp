#pragma once
#include <cstdint>

// svcc:c2rtl
uint32_t mac16_golden(uint16_t a, uint16_t b, uint32_t c) {
  uint32_t prod = a * b;
  uint32_t sum = prod + c;
  return sum;
}
