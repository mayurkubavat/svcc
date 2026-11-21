# `uvir` Milestone & Capability Status

| Component / Dialect | `sv-uvm` (1800-2023) | `sc-uvm` (1666-2023 + CRAVE + FC4SC) | Multi-Language Hub (`TargetCapabilities`) | Status |
| :--- | :--- | :--- | :--- | :--- |
| `types` (2/4-state, widths, signedness) | Lower + Emit | Lower + Emit (`uint*_t`, `sc_uint`, `sc_lv`) | `WidthSignednessPass` + `OwnershipPass` | Verified (M0) |
| `uvm` (`sequence_item`, `reg`, field ops) | Lower + Emit (`svlang` + RAL) | Lower + Emit (`uvm_randomized_*`, copy-ctor) | `FieldOpSynthesisPass` | Verified (M0) |
| `crv` (hard/soft, `->`, `inside`, `[v +/- t]`, `unique`) | Lower + Emit | Lower + Emit (`crv_constraint`, `if_then`) | `TargetLegalizerPass` + Z3 `QF_BV` proof | Verified (M0) |
| `cov` (`covergroup`, `coverpoint`, `bins`, `cross`) | Lower + Emit | Lower + Emit (`fc4sc::covergroup`) | UCIS 1.0 isomorphism checker | Verified (M0) |
| `sched` & `sva` (`fork/disable`, `#1step`, temporal) | IR + Roles | IR + Lowering plan | `ConcurrencyModel` & `TemporalModel` profiles | M1 Next |
| Additional Spokes (`py-uvm`, `pss`, `rust-dv`) | — | — | `LanguageRegistry` + capability presets ready | M2 Planned |
