#pragma once
// Canonical SystemC 3.0.3 + UVM-SystemC + CRAVE + FC4SC counterpart fixture
#include <cstdint>
#include <uvm>
#include <crave2/crave.hpp>
#include <fc4sc.hpp>

class apb_timer_item : public uvm::uvm_randomized_sequence_item {
public:
  UVM_OBJECT_UTILS(apb_timer_item);

  crave::crv_variable<uint32_t> paddr{"paddr"};
  crave::crv_variable<uint32_t> pwdata{"pwdata"};
  crave::crv_variable<sc_dt::sc_uint<4>> pstrb{"pstrb"};
  crave::crv_variable<uint8_t> burst_len{"burst_len"};
  crave::crv_variable<bool> pwrite{"pwrite"};
  uint32_t seq_tag = 0;

  crave::crv_constraint c_apb_protocol{this, "c_apb_protocol",
    ((paddr() & 3U) == 0U),
    ((paddr() >= 0x40000000U && paddr() <= 0x40000FFFU) || (paddr() >= (0x40010100U - 0x10U) && paddr() <= (0x40010100U + 0x10U))),
    crave::if_then(pwrite(), (pstrb() != 0U)),
    crave::if_then((!pwrite()), (pstrb() == 0U)),
    ((burst_len() == 1U) || (burst_len() == 4U) || (burst_len() == 8U) || (burst_len() == 16U))
  };
  crave::crv_soft_constraint c_apb_protocol__soft_0{this, "c_apb_protocol__soft_0", (burst_len() == 4U)};

  explicit apb_timer_item(const std::string& name = "apb_timer_item")
      : uvm::uvm_randomized_sequence_item(name) {
    set_sv_signedness(true);
  }
};

class cg_apb_timer : public fc4sc::covergroup {
public:
  apb_timer_item sample_Point{};
  CG_CONS(cg_apb_timer) {}
  COVERPOINT(uint32_t, cp_addr, sample_Point.paddr) {
    fc4sc::bin<uint32_t>("timer0_regs", fc4sc::interval(0x40000000U, 0x400007FFU)),
    fc4sc::bin<uint32_t>("timer1_regs", fc4sc::interval(0x40000800U, 0x40000FFFU))
  };
  COVERPOINT(bool, cp_dir, sample_Point.pwrite) {
    fc4sc::bin<bool>("rd", 0),
    fc4sc::bin<bool>("wr", 1)
  };
  COVERPOINT(uint8_t, cp_burst, sample_Point.burst_len) {
    fc4sc::bin<uint8_t>("single", 1),
    fc4sc::bin<uint8_t>("incr4", 4),
    fc4sc::bin<uint8_t>("incr8", 8),
    fc4sc::bin<uint8_t>("incr16", 16)
  };
  fc4sc::cross<uint32_t, bool> cx_dir_addr{this, "cx_dir_addr", &cp_addr, &cp_dir};
};
