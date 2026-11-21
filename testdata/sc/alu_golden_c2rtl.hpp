#pragma once
// Untimed C++17 Golden Architectural Reference Function for C2RTL / InFormal SEC (`latency = 2` vs `alu_pipe`)
#include <cstdint>
#include <systemc>

// svcc:c2rtl
uint32_t alu_golden(sc_dt::sc_uint<2> op, uint32_t a, uint32_t b) {
  uint32_t acc = a + b;
  if (op == 1U) {
    acc = a - b;
  }
  if (op == 2U) {
    acc = a & b;
  }
  if (op == 3U) {
    acc = a ^ b;
  }
  return acc;
}
