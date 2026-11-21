#pragma once
#include <cstdint>

// svcc:c2rtl
uint8_t crc8_step_golden(uint8_t crc_in, uint8_t data_in) {
  uint8_t mixed = crc_in ^ data_in;
  uint8_t out = mixed >> 1U;
  if ((mixed & 1U) == 1U) {
    out = (mixed >> 1U) ^ 0x8CU;
  }
  return out;
}
