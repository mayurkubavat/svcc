class eth_mac_item extends uvm_sequence_item;
  rand bit [15:0] ethertype;
  rand bit [13:0] payload_bytes;
  rand bit        vlan_en;
  rand bit [11:0] vlan_vid;
  bit [31:0]      fcs_crc;

  `uvm_object_utils_begin(eth_mac_item)
    `uvm_field_int(ethertype, UVM_ALL_ON)
    `uvm_field_int(payload_bytes, UVM_ALL_ON)
    `uvm_field_int(vlan_en, UVM_ALL_ON)
    `uvm_field_int(vlan_vid, UVM_ALL_ON)
    `uvm_field_int(fcs_crc, UVM_NOCOMPARE)
  `uvm_object_utils_end

  constraint c_ieee_802_3 {
    payload_bytes inside {[14'd46 : 14'd1500], [14'd9000 +/- 14'd16]};
    vlan_en -> (vlan_vid inside {[12'd1 : 12'd4094]});
    (!vlan_en) -> (vlan_vid == 12'd0);
    ethertype inside {16'h0800, 16'h0806, 16'h86DD};
    soft payload_bytes == 14'd64;
  }

  covergroup cg_eth_frame;
    cp_size: coverpoint payload_bytes {
      bins min_pkt   = {46};
      bins std_frame = {[47 : 1500]};
      bins jumbo     = {[8984 : 9016]};
    }
    cp_vlan: coverpoint vlan_en {
      bins untagged = {0};
      bins tagged_v = {1};
    }
    cx_vlan_sz: cross cp_size, cp_vlan;
  endgroup

  function new(string name = "eth_mac_item");
    super.new(name);
    cg_eth_frame = new();
  endfunction
endclass
