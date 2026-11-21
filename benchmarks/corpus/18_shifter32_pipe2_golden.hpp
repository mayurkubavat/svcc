#pragma once
#include <cstdint>
#include <systemc>

// svcc:c2rtl
uint32_t shifter32_golden(uint32_t din, sc_dt::sc_uint<5> shamt, bool dir_right) {
  uint32_t out = din << shamt;
  if (dir_right) {
    out = din >> shamt;
  }
  return out;
}
