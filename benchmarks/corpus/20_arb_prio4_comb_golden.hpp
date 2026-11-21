#pragma once
#include <systemc>

// svcc:c2rtl
sc_dt::sc_uint<4> arb_prio4_golden(sc_dt::sc_uint<4> req) {
  sc_dt::sc_uint<4> gnt = 0U;
  if ((req & 8U) != 0U) {
    gnt = 8U;
  }
  if ((req & 4U) != 0U) {
    gnt = 4U;
  }
  if ((req & 2U) != 0U) {
    gnt = 2U;
  }
  if ((req & 1U) != 0U) {
    gnt = 1U;
  }
  return gnt;
}
