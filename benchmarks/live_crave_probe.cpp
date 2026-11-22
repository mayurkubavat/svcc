// Empirical live C++17 probe against installed Accellera SystemC + UVM-SystemC + CRAVE + FC4SC
#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <numeric>
#include <set>
#include <sstream>
#include <string>
#include <vector>

#include <systemc>
#include <uvm>
#include <crave/ConstrainedRandom.hpp>
#include <crave/experimental/Experimental.hpp>
#include <crave/experimental/SystemC.hpp>
#include <uvm_randomized_sequence_item.h>
#include <fc4sc.hpp>

// Thin adapter demonstrating how svcc's `.add` / `.add_range` / `.add_weighted_range`
// maps onto real Accellera CRAVE's `crave::distribution<T>` + `weighted_range<T>` / `weighted_value<T>`.
template <typename T>
struct svcc_crave_dist_builder {
  crave::distribution<T> d;
  svcc_crave_dist_builder& add(T val, uint64_t w) {
    d(crave::weighted_value<T>(val, w));
    return *this;
  }
  svcc_crave_dist_builder& add_range(T lo, T hi, uint64_t w_per_val) {
    uint64_t span = static_cast<uint64_t>(hi) - static_cast<uint64_t>(lo) + 1ULL;
    d(crave::weighted_range<T>(lo, hi, span * w_per_val));
    return *this;
  }
  svcc_crave_dist_builder& add_weighted_range(T lo, T hi, uint64_t w_split) {
    d(crave::weighted_range<T>(lo, hi, w_split));
    return *this;
  }
  crave::distribution_tag<T> build() const {
    return crave::distribution_tag<T>(d);
  }
};

// O(1)-memory cycle-walking 4-round Feistel permutation for W-bit randc (`svcc_randc_feistel<W>`)
template <unsigned W>
class svcc_randc_feistel {
  static_assert(W >= 1 && W <= 30, "W must be in [1, 30]");
  static constexpr unsigned HALF = (W + 1U) / 2U;
  static constexpr unsigned EVEN_W = HALF * 2U;
  static constexpr uint32_t HALF_MASK = (1U << HALF) - 1U;
  static constexpr uint32_t DOMAIN_SIZE = (1U << W);

  uint32_t idx_ = 0;
  uint32_t seed_ = 0x9E3779B9U;

  static inline uint32_t round_fn(uint32_t r, uint32_t k) {
    uint32_t x = (r ^ k) * 0x45D9F3BU;
    x = ((x >> 16) ^ x) * 0x45D9F3BU;
    x = (x >> 16) ^ x;
    return x & HALF_MASK;
  }

  uint32_t permute_even(uint32_t x) const {
    uint32_t l = (x >> HALF) & HALF_MASK;
    uint32_t r = x & HALF_MASK;
    for (unsigned rnd = 0; rnd < 4U; ++rnd) {
      uint32_t rk = (seed_ + rnd * 0xBF58476DU) & HALF_MASK;
      uint32_t nl = r;
      uint32_t nr = (l ^ round_fn(r, rk)) & HALF_MASK;
      l = nl;
      r = nr;
    }
    return (l << HALF) | r;
  }

public:
  explicit svcc_randc_feistel(uint32_t seed = 0x9E3779B9U) : idx_(0), seed_(seed) {}

  void reseed(uint32_t new_seed) {
    idx_ = 0;
    seed_ = new_seed;
  }

  uint32_t next() {
    if (idx_ >= DOMAIN_SIZE) {
      seed_ = seed_ * 1664525U + 1013904223U;
      idx_ = 0;
    }
    uint32_t x = idx_++;
    // Cycle-walking guarantees exact bijection on [0, 2^W - 1] even if W is odd
    do {
      x = permute_even(x);
    } while (x >= DOMAIN_SIZE);
    return x;
  }
};

// ============================================================================
// 1. Live UVM-SystemC + CRAVE + FC4SC Verification (corrected C++17 emission)
// ============================================================================
class apb_timer_item : public ::uvm_randomized_sequence_item {
public:
  UVM_OBJECT_UTILS(apb_timer_item);

  crave::crv_variable<uint32_t> paddr{"paddr"};
  crave::crv_variable<uint32_t> pwdata{"pwdata"};
  crave::crv_variable<sc_dt::sc_uint<4>> pstrb{"pstrb"};
  crave::crv_variable<uint8_t> burst_len{"burst_len"};
  crave::crv_variable<bool> pwrite{"pwrite"};
  uint32_t seq_tag = 0;

  crave::crv_constraint c_apb_protocol{
    "c_apb_protocol",
    ((paddr() & 3U) == 0U),
    ((paddr() >= 0x40000000U && paddr() <= 0x40000FFFU) ||
     (paddr() >= (0x40010100U - 0x10U) && paddr() <= (0x40010100U + 0x10U))),
    crave::if_then(pwrite(), (pstrb() != sc_dt::sc_uint<4>(0))),
    crave::if_then((!pwrite()), (pstrb() == sc_dt::sc_uint<4>(0))),
    crave::dist(
      burst_len(),
      crave::distribution<uint8_t>::create(crave::weighted_value<uint8_t>(1, 40))
                                          (crave::weighted_value<uint8_t>(4, 30))
                                          (crave::weighted_value<uint8_t>(8, 20))
                                          (crave::weighted_value<uint8_t>(16, 10))
    )
  };
  crave::crv_soft_constraint c_apb_protocol__soft_0{
    "c_apb_protocol__soft_0",
    (paddr() <= 0x40000FFFU)
  };

  apb_timer_item(crave::crv_object_name name = "apb_timer_item")
      : ::uvm_randomized_sequence_item(name) {}

  void do_copy(const uvm::uvm_object& rhs) override {
    const auto* rhs_ = dynamic_cast<const apb_timer_item*>(&rhs);
    if (!rhs_) return;
    this->paddr = static_cast<uint32_t>(rhs_->paddr);
    this->pwdata = static_cast<uint32_t>(rhs_->pwdata);
    this->pstrb = static_cast<sc_dt::sc_uint<4>>(rhs_->pstrb);
    this->burst_len = static_cast<uint8_t>(rhs_->burst_len);
    this->pwrite = static_cast<bool>(rhs_->pwrite);
    this->seq_tag = rhs_->seq_tag;
  }

  bool do_compare(const uvm::uvm_object& rhs, const uvm::uvm_comparer*) const override {
    const auto* rhs_ = dynamic_cast<const apb_timer_item*>(&rhs);
    if (!rhs_) return false;
    return (static_cast<uint32_t>(this->paddr) == static_cast<uint32_t>(rhs_->paddr)) &&
           (static_cast<uint32_t>(this->pwdata) == static_cast<uint32_t>(rhs_->pwdata)) &&
           (static_cast<sc_dt::sc_uint<4>>(this->pstrb) == static_cast<sc_dt::sc_uint<4>>(rhs_->pstrb)) &&
           (static_cast<uint8_t>(this->burst_len) == static_cast<uint8_t>(rhs_->burst_len)) &&
           (static_cast<bool>(this->pwrite) == static_cast<bool>(rhs_->pwrite));
  }
};

struct apb_timer_cov_sample {
  uint32_t paddr = 0;
  bool pwrite = false;
  uint8_t burst_len = 0;
};

class cg_apb_timer : public fc4sc::covergroup {
public:
  apb_timer_cov_sample sample_Point{};
  CG_CONS(cg_apb_timer) {}

  void sample(const apb_timer_item& item_in) {
    sample_Point.paddr = static_cast<uint32_t>(item_in.paddr);
    sample_Point.pwrite = static_cast<bool>(item_in.pwrite);
    sample_Point.burst_len = static_cast<uint8_t>(item_in.burst_len);
    fc4sc::covergroup::sample();
  }

  COVERPOINT(uint32_t, cp_addr, sample_Point.paddr) {
    fc4sc::bin<uint32_t>("timer0_regs", fc4sc::interval<uint32_t>(0x40000000U, 0x400007FFU)),
    fc4sc::bin<uint32_t>("timer1_regs", fc4sc::interval<uint32_t>(0x40000800U, 0x40000FFFU)),
    fc4sc::ignore_bin<uint32_t>("unmapped", fc4sc::interval<uint32_t>(0x0U, 0x3FFFFFFFU))
  };
  COVERPOINT(bool, cp_dir, sample_Point.pwrite) {
    fc4sc::bin<bool>("rd", false),
    fc4sc::bin<bool>("wr", true)
  };
  COVERPOINT(uint8_t, cp_burst, sample_Point.burst_len) {
    fc4sc::bin<uint8_t>("single", static_cast<uint8_t>(1)),
    fc4sc::bin<uint8_t>("incr4", static_cast<uint8_t>(4)),
    fc4sc::bin<uint8_t>("incr8", static_cast<uint8_t>(8)),
    fc4sc::bin<uint8_t>("incr16", static_cast<uint8_t>(16))
  };
  fc4sc::cross<uint32_t, bool> cx_dir_addr{this, "cx_dir_addr", &cp_addr, &cp_dir};
};

static double g_uvm_fc4sc_inst_cov = 0.0;
static uint32_t g_uvm_rand_ok = 0;
static bool g_uvm_soft_override_ok = false;

class probe_uvm_test : public uvm::uvm_test {
public:
  UVM_COMPONENT_UTILS(probe_uvm_test);
  explicit probe_uvm_test(uvm::uvm_component_name name) : uvm::uvm_test(name) {}

  void run_phase(uvm::uvm_phase& phase) override {
    phase.raise_objection(this);
    auto* item = apb_timer_item::type_id::create("item");
    cg_apb_timer cg("cg_apb_timer_inst");
    for (int i = 0; i < 200; ++i) {
      if (item->randomize()) {
        ++g_uvm_rand_ok;
        cg.sample(*item);
      }
    }
    // Also verify soft constraint override via randomize_with
    g_uvm_soft_override_ok = item->randomize_with(item->paddr() >= 0x400100F0U) &&
                             (static_cast<uint32_t>(item->paddr) >= 0x400100F0U);
    g_uvm_fc4sc_inst_cov = cg.get_inst_coverage();
    sc_core::wait(10, sc_core::SC_NS);
    phase.drop_objection(this);
  }
};

// ============================================================================
// 2. Prediction P2: 100,000 draws in live Accellera CRAVE on :/ + := dists
// ============================================================================
struct DistMetrics {
  std::vector<uint64_t> counts;
  double tvd_vs_ieee = 0.0;
  double jsd_bits_vs_ieee = 0.0;
  double chi2_vs_ieee = 0.0;
  double elapsed_ms = 0.0;
};

static DistMetrics compute_dist_metrics(
    const std::vector<uint64_t>& counts,
    const std::vector<double>& target_probs,
    uint64_t n_draws,
    double elapsed_ms) {
  DistMetrics m;
  m.counts = counts;
  m.elapsed_ms = elapsed_ms;
  double tvd = 0.0;
  double jsd_nats = 0.0;
  double chi2 = 0.0;
  for (size_t i = 0; i < counts.size(); ++i) {
    double p = static_cast<double>(counts[i]) / static_cast<double>(n_draws);
    double q = target_probs[i];
    tvd += 0.5 * std::fabs(p - q);
    double mid = 0.5 * (p + q);
    if (p > 0.0 && mid > 0.0) jsd_nats += 0.5 * p * std::log(p / mid);
    if (q > 0.0 && mid > 0.0) jsd_nats += 0.5 * q * std::log(q / mid);
    if (q > 0.0) {
      double exp_c = q * static_cast<double>(n_draws);
      double diff = static_cast<double>(counts[i]) - exp_c;
      chi2 += (diff * diff) / exp_c;
    }
  }
  m.tvd_vs_ieee = tvd;
  m.jsd_bits_vs_ieee = jsd_nats / std::log(2.0);
  m.chi2_vs_ieee = chi2;
  return m;
}

static double two_sample_tvd(const std::vector<uint64_t>& a, const std::vector<uint64_t>& b, uint64_t n) {
  double tvd = 0.0;
  for (size_t i = 0; i < a.size(); ++i) {
    double pa = static_cast<double>(a[i]) / static_cast<double>(n);
    double pb = static_cast<double>(b[i]) / static_cast<double>(n);
    tvd += 0.5 * std::fabs(pa - pb);
  }
  return tvd;
}

struct P2Item : public crave::crv_sequence_item {
  crave::crv_variable<uint32_t> x{"x"};
  crave::crv_constraint c_dist;

  P2Item(crave::crv_object_name name, uint32_t r_lo, uint32_t r_hi, uint64_t r_w, uint32_t s_val, uint64_t s_w)
      : c_dist(
            "c_dist",
            crave::dist(
                x(),
                crave::distribution<uint32_t>::create(crave::weighted_range<uint32_t>(r_lo, r_hi, r_w))
                                                     (crave::weighted_value<uint32_t>(s_val, s_w)))) {
    (void)name;
  }
};

struct P2ConstrainedItem : public crave::crv_sequence_item {
  crave::crv_variable<uint32_t> x{"x"};
  crave::crv_constraint c_dist;

  P2ConstrainedItem(crave::crv_object_name name, uint32_t r_lo, uint32_t r_hi, uint64_t r_w, uint32_t s_val, uint64_t s_w, uint32_t min_x)
      : c_dist(
            "c_dist",
            crave::dist(
                x(),
                crave::distribution<uint32_t>::create(crave::weighted_range<uint32_t>(r_lo, r_hi, r_w))
                                                     (crave::weighted_value<uint32_t>(s_val, s_w))),
            (x() >= min_x)) {
    (void)name;
  }
};

static DistMetrics run_p2_case(
    uint32_t r_lo, uint32_t r_hi, uint64_t r_w,
    uint32_t s_val, uint64_t s_w,
    const std::vector<double>& target_probs,
    uint64_t n_draws,
    unsigned seed) {
  crave::set_global_seed(seed);
  P2Item it("p2_item", r_lo, r_hi, r_w, s_val, s_w);
  std::vector<uint64_t> counts(target_probs.size(), 0ULL);
  auto t0 = std::chrono::steady_clock::now();
  for (uint64_t i = 0; i < n_draws; ++i) {
    if (!it.randomize()) {
      throw std::runtime_error("P2Item randomize() failed");
    }
    uint32_t v = static_cast<uint32_t>(it.x);
    if (v < counts.size()) ++counts[v];
  }
  double ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count();
  return compute_dist_metrics(counts, target_probs, n_draws, ms);
}

static DistMetrics run_p2_constrained_case(
    uint32_t r_lo, uint32_t r_hi, uint64_t r_w,
    uint32_t s_val, uint64_t s_w,
    uint32_t min_x,
    const std::vector<double>& target_probs,
    uint64_t n_draws,
    unsigned seed) {
  crave::set_global_seed(seed);
  P2ConstrainedItem it("p2_c_item", r_lo, r_hi, r_w, s_val, s_w, min_x);
  std::vector<uint64_t> counts(target_probs.size(), 0ULL);
  auto t0 = std::chrono::steady_clock::now();
  for (uint64_t i = 0; i < n_draws; ++i) {
    if (!it.randomize()) {
      throw std::runtime_error("P2ConstrainedItem randomize() failed");
    }
    uint32_t v = static_cast<uint32_t>(it.x);
    if (v < counts.size()) ++counts[v];
  }
  double ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count();
  return compute_dist_metrics(counts, target_probs, n_draws, ms);
}

// ============================================================================
// 3. Prediction P5: History-exclusion randc vs native CRAVE randc vs Feistel
// ============================================================================
struct P5Result {
  unsigned width = 0;
  uint32_t domain_size = 0;
  uint32_t unique_values = 0;
  double total_ms = 0.0;
  double mean_us_per_call = 0.0;
  double p50_us_per_call = 0.0;
  double p99_us_per_call = 0.0;
  double last_step_us = 0.0;
};

static P5Result bench_crave_history_exclusion(unsigned w) {
  struct HistItem : public crave::crv_sequence_item {
    crave::crv_variable<uint32_t> x{"x"};
    crave::crv_constraint c_bound;
    crave::crv_constraint c_excl{"c_excl"};
    HistItem(crave::crv_object_name name, unsigned width)
        : c_bound("c_bound", x() < (1U << width)) {
      (void)name;
    }
  };

  crave::set_global_seed(100 + w);
  HistItem item("hist_item", w);
  const uint32_t n = (1U << w);
  std::vector<double> call_us;
  call_us.reserve(n);
  std::set<uint32_t> seen;

  auto t_all0 = std::chrono::steady_clock::now();
  for (uint32_t m = 0; m < n; ++m) {
    auto t0 = std::chrono::steady_clock::now();
    if (!item.randomize()) {
      throw std::runtime_error("HistItem randomize() failed at m=" + std::to_string(m));
    }
    double us = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t0).count();
    call_us.push_back(us);
    uint32_t val = static_cast<uint32_t>(item.x);
    seen.insert(val);
    if (m + 1 < n) {
      auto t_rb0 = std::chrono::steady_clock::now();
      item.c_excl &= crave::crv_constraint("tmp_excl", item.x() != val);
      double rb_us = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t_rb0).count();
      call_us.back() += rb_us;
    }
  }
  double total_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t_all0).count();

  double last_us = call_us.back();
  std::vector<double> sorted = call_us;
  std::sort(sorted.begin(), sorted.end());

  P5Result r;
  r.width = w;
  r.domain_size = n;
  r.unique_values = static_cast<uint32_t>(seen.size());
  r.total_ms = total_ms;
  r.mean_us_per_call = (total_ms * 1000.0) / static_cast<double>(n);
  r.p50_us_per_call = sorted[n / 2];
  r.p99_us_per_call = sorted[static_cast<size_t>(std::min<uint32_t>(n - 1, (n * 99U) / 100U))];
  r.last_step_us = last_us;
  return r;
}

template <unsigned W>
static P5Result bench_crave_native_randc() {
  struct NativeRandcItem : public crave::crv_sequence_item {
    crave::crv_variable<sc_dt::sc_uint<W>> x{"x"};
    explicit NativeRandcItem(crave::crv_object_name name = "native_randc_item") {
      (void)name;
      x.randc(true);
    }
  };

  crave::set_global_seed(200 + W);
  NativeRandcItem item("native_randc_item");
  const uint32_t n = (1U << W);
  std::vector<double> call_us;
  call_us.reserve(n);
  std::vector<uint8_t> seen(n, 0);
  uint32_t uniq = 0;

  auto t_all0 = std::chrono::steady_clock::now();
  for (uint32_t m = 0; m < n; ++m) {
    auto t0 = std::chrono::steady_clock::now();
    if (!item.randomize()) {
      throw std::runtime_error("NativeRandcItem randomize() failed");
    }
    double us = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t0).count();
    call_us.push_back(us);
    uint32_t v = static_cast<sc_dt::sc_uint<W>>(item.x).to_uint();
    if (!seen[v]) {
      seen[v] = 1;
      ++uniq;
    }
  }
  double total_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t_all0).count();
  double last_us = call_us.back();
  std::sort(call_us.begin(), call_us.end());

  P5Result r;
  r.width = W;
  r.domain_size = n;
  r.unique_values = uniq;
  r.total_ms = total_ms;
  r.mean_us_per_call = (total_ms * 1000.0) / static_cast<double>(n);
  r.p50_us_per_call = call_us[n / 2];
  r.p99_us_per_call = call_us[static_cast<size_t>(std::min<uint32_t>(n - 1, (n * 99U) / 100U))];
  r.last_step_us = last_us;
  return r;
}

template <unsigned W>
static P5Result bench_feistel_pure() {
  svcc_randc_feistel<W> gen(0x9E3779B9U ^ W);
  const uint32_t n = (1U << W);
  std::vector<uint8_t> seen(n, 0);
  uint32_t uniq = 0;

  // Measure batch timing cleanly (repeat if W is tiny so steady_clock resolution is exact)
  const uint32_t reps = (n < 4096U) ? (4096U / n) : 1U;
  auto t0 = std::chrono::steady_clock::now();
  volatile uint32_t sink = 0;
  for (uint32_t rep = 0; rep < reps; ++rep) {
    for (uint32_t m = 0; m < n; ++m) {
      uint32_t v = gen.next();
      sink ^= v;
      if (rep == 0 && !seen[v]) {
        seen[v] = 1;
        ++uniq;
      }
    }
  }
  double elapsed_us = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t0).count();
  (void)sink;

  double per_call_us = elapsed_us / static_cast<double>(n * reps);
  P5Result r;
  r.width = W;
  r.domain_size = n;
  r.unique_values = uniq;
  r.total_ms = (per_call_us * static_cast<double>(n)) / 1000.0;
  r.mean_us_per_call = per_call_us;
  r.p50_us_per_call = per_call_us;
  r.p99_us_per_call = per_call_us;
  r.last_step_us = per_call_us;
  return r;
}

template <unsigned W>
static P5Result bench_feistel_in_crave_item() {
  struct FeistelCraveItem : public crave::crv_sequence_item {
    svcc_randc_feistel<W> feistel{0x9E3779B9U ^ W};
    crave::crv_variable<uint32_t> x{"x"};
    crave::crv_variable<uint32_t> y{"y"};
    crave::crv_constraint c_dep{"c_dep", y() == (x() ^ 0x5AU)};
    explicit FeistelCraveItem(crave::crv_object_name name = "feistel_item") {
      (void)name;
      x.rand_mode(false);
    }
    void pre_randomize() override { x = feistel.next(); }
  };

  crave::set_global_seed(300 + W);
  FeistelCraveItem item("feistel_item");
  const uint32_t n = (1U << W);
  std::vector<double> call_us;
  call_us.reserve(n);
  std::vector<uint8_t> seen(n, 0);
  uint32_t uniq = 0;

  auto t_all0 = std::chrono::steady_clock::now();
  for (uint32_t m = 0; m < n; ++m) {
    auto t0 = std::chrono::steady_clock::now();
    if (!item.randomize()) {
      throw std::runtime_error("FeistelCraveItem randomize() failed");
    }
    double us = std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now() - t0).count();
    call_us.push_back(us);
    uint32_t v = static_cast<uint32_t>(item.x);
    if (!seen[v]) {
      seen[v] = 1;
      ++uniq;
    }
  }
  double total_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t_all0).count();
  double last_us = call_us.back();
  std::sort(call_us.begin(), call_us.end());

  P5Result r;
  r.width = W;
  r.domain_size = n;
  r.unique_values = uniq;
  r.total_ms = total_ms;
  r.mean_us_per_call = (total_ms * 1000.0) / static_cast<double>(n);
  r.p50_us_per_call = call_us[n / 2];
  r.p99_us_per_call = call_us[static_cast<size_t>(std::min<uint32_t>(n - 1, (n * 99U) / 100U))];
  r.last_step_us = last_us;
  return r;
}

static std::string vec_u64_json(const std::vector<uint64_t>& v) {
  std::ostringstream oss;
  oss << "[";
  for (size_t i = 0; i < v.size(); ++i) {
    if (i) oss << ", ";
    oss << v[i];
  }
  oss << "]";
  return oss.str();
}

static std::string p5_row_json(const P5Result& r) {
  std::ostringstream oss;
  oss << std::fixed << std::setprecision(4)
      << "{"
      << "\"W\": " << r.width << ", "
      << "\"domain_size\": " << r.domain_size << ", "
      << "\"unique_values\": " << r.unique_values << ", "
      << "\"total_ms\": " << r.total_ms << ", "
      << "\"mean_us_per_call\": " << r.mean_us_per_call << ", "
      << "\"p50_us_per_call\": " << r.p50_us_per_call << ", "
      << "\"p99_us_per_call\": " << r.p99_us_per_call << ", "
      << "\"last_step_us\": " << r.last_step_us
      << "}";
  return oss.str();
}

int sc_main(int argc, char* argv[]) {
  (void)argc;
  (void)argv;
  crave::logging_severity = crave::ERROR;
  crave::set_solver_backend("Z3");
  crave::set_sv_signedness(true);
  crave::set_global_seed(42);

  std::cout << "[1/3] Running live SystemC + UVM-SystemC + CRAVE + FC4SC test..." << std::endl;
  uvm::run_test("probe_uvm_test");
  std::cout << "  -> UVM randomize_ok=" << g_uvm_rand_ok
            << " soft_override_ok=" << (g_uvm_soft_override_ok ? "true" : "false")
            << " fc4sc_inst_cov=" << g_uvm_fc4sc_inst_cov << "%" << std::endl;

  std::cout << "[2/3] Running Prediction P2 (100,000 real CRAVE randomize() draws per dist)..." << std::endl;
  const uint64_t N_P2 = 100000ULL;

  // Case 1: x dist { [0:2] :/ 10, 3 := 2 }
  std::vector<double> p2_c1_target = {10.0 / 36.0, 10.0 / 36.0, 10.0 / 36.0, 6.0 / 36.0};
  auto p2_c1_unscaled = run_p2_case(0, 2, 10, 3, 2, p2_c1_target, N_P2, 12345);
  auto p2_c1_lcm = run_p2_case(0, 2, 30, 3, 6, p2_c1_target, N_P2, 12345);
  // Naive integer-floor per-value weight (10/3 = 3 -> range weight 3*3 = 9 vs 2)
  auto p2_c1_int_floor = run_p2_case(0, 2, 9, 3, 2, p2_c1_target, N_P2, 12345);
  // Intersecting hard constraint x >= 1 on [0:2] :/ 10, 3 := 2 (IEEE target: 0, 5/13, 5/13, 3/13)
  std::vector<double> p2_c1_cond_target = {0.0, 5.0 / 13.0, 5.0 / 13.0, 3.0 / 13.0};
  auto p2_c1_cond_unscaled = run_p2_constrained_case(0, 2, 10, 3, 2, 1, p2_c1_cond_target, N_P2, 12345);
  auto p2_c1_cond_lcm = run_p2_constrained_case(0, 2, 30, 3, 6, 1, p2_c1_cond_target, N_P2, 12345);

  // Case 2: x dist { [0:15] :/ 5, 16 := 5 }
  std::vector<double> p2_c2_target(17, 1.0 / 32.0);
  p2_c2_target[16] = 0.5;
  auto p2_c2_unscaled = run_p2_case(0, 15, 5, 16, 5, p2_c2_target, N_P2, 54321);
  auto p2_c2_lcm = run_p2_case(0, 15, 80, 16, 80, p2_c2_target, N_P2, 54321);
  // Naive integer-floor per-value weight (5/16 = 0 -> range weight 0 vs 5)
  auto p2_c2_int_floor = run_p2_case(0, 15, 0, 16, 5, p2_c2_target, N_P2, 54321);
  // Intersecting hard constraint x >= 8 on [0:15] :/ 5, 16 := 5 (IEEE target: 8..15 each 1/24, 16 is 2/3)
  std::vector<double> p2_c2_cond_target(17, 0.0);
  for (size_t i = 8; i < 16; ++i) p2_c2_cond_target[i] = 1.0 / 24.0;
  p2_c2_cond_target[16] = 2.0 / 3.0;
  auto p2_c2_cond_unscaled = run_p2_constrained_case(0, 15, 5, 16, 5, 8, p2_c2_cond_target, N_P2, 54321);
  auto p2_c2_cond_lcm = run_p2_constrained_case(0, 15, 80, 16, 80, 8, p2_c2_cond_target, N_P2, 54321);

  std::cout << "  -> Case1 [0:2]:/10, 3:=2 | unscaled TVD=" << p2_c1_unscaled.tvd_vs_ieee
            << " chi2=" << p2_c1_unscaled.chi2_vs_ieee
            << " | LCM TVD=" << p2_c1_lcm.tvd_vs_ieee
            << " chi2=" << p2_c1_lcm.chi2_vs_ieee
            << " | int-floor TVD=" << p2_c1_int_floor.tvd_vs_ieee
            << " | constrained(x>=1) TVD=" << p2_c1_cond_unscaled.tvd_vs_ieee << std::endl;
  std::cout << "  -> Case2 [0:15]:/5, 16:=5 | unscaled TVD=" << p2_c2_unscaled.tvd_vs_ieee
            << " chi2=" << p2_c2_unscaled.chi2_vs_ieee
            << " | LCM TVD=" << p2_c2_lcm.tvd_vs_ieee
            << " chi2=" << p2_c2_lcm.chi2_vs_ieee
            << " | int-floor TVD=" << p2_c2_int_floor.tvd_vs_ieee
            << " | constrained(x>=8) TVD=" << p2_c2_cond_unscaled.tvd_vs_ieee << std::endl;

  std::cout << "[3/3] Running Prediction P5 (CRAVE history-exclusion vs native randc vs Feistel)..." << std::endl;
  std::vector<P5Result> p5_hist;
  for (unsigned w : {4U, 6U, 8U, 10U}) {
    auto r = bench_crave_history_exclusion(w);
    p5_hist.push_back(r);
    std::cout << "  -> CRAVE history-excl W=" << w << " (2^W=" << r.domain_size << ")"
              << " unique=" << r.unique_values
              << " mean=" << r.mean_us_per_call << " us/call"
              << " last_step=" << r.last_step_us << " us"
              << " total=" << r.total_ms << " ms" << std::endl;
  }

  // Verify CRAVE native randc width limit (>16 bits throws)
  bool native_gt16_throws = false;
  std::string native_gt16_err;
  try {
    crave::crv_variable<sc_dt::sc_uint<17>> x17{"x17"};
    x17.randc(true);
  } catch (const std::exception& e) {
    native_gt16_throws = true;
    native_gt16_err = e.what();
  }
  std::cout << "  -> CRAVE native randc W=17 throws=" << (native_gt16_throws ? "true" : "false")
            << " (" << native_gt16_err << ")" << std::endl;

  std::vector<P5Result> p5_native = {
    bench_crave_native_randc<4>(),
    bench_crave_native_randc<8>(),
    bench_crave_native_randc<10>(),
    bench_crave_native_randc<12>(),
    bench_crave_native_randc<16>()
  };
  for (const auto& r : p5_native) {
    std::cout << "  -> CRAVE native randc W=" << r.width
              << " unique=" << r.unique_values
              << " mean=" << r.mean_us_per_call << " us/call"
              << " total=" << r.total_ms << " ms" << std::endl;
  }

  std::vector<P5Result> p5_feistel_pure = {
    bench_feistel_pure<4>(),
    bench_feistel_pure<8>(),
    bench_feistel_pure<10>(),
    bench_feistel_pure<12>(),
    bench_feistel_pure<16>()
  };
  for (const auto& r : p5_feistel_pure) {
    std::cout << "  -> svcc_randc_feistel<W> (pure) W=" << r.width
              << " unique=" << r.unique_values
              << " mean=" << r.mean_us_per_call << " us/call" << std::endl;
  }

  std::vector<P5Result> p5_feistel_crave = {
    bench_feistel_in_crave_item<4>(),
    bench_feistel_in_crave_item<8>(),
    bench_feistel_in_crave_item<10>(),
    bench_feistel_in_crave_item<12>(),
    bench_feistel_in_crave_item<16>()
  };
  for (const auto& r : p5_feistel_crave) {
    std::cout << "  -> svcc_randc_feistel<W> + CRAVE item W=" << r.width
              << " unique=" << r.unique_values
              << " mean=" << r.mean_us_per_call << " us/call"
              << " total=" << r.total_ms << " ms" << std::endl;
  }

  // Write live_crave_results.json
  std::ofstream out("live_crave_results.json");
  out << std::fixed << std::setprecision(6);
  out << "{\n";
  out << "  \"live_uvm_fc4sc_smoke\": {\n";
  out << "    \"randomize_draws\": " << g_uvm_rand_ok << ",\n";
  out << "    \"soft_constraint_override_ok\": " << (g_uvm_soft_override_ok ? "true" : "false") << ",\n";
  out << "    \"fc4sc_inst_coverage_pct\": " << g_uvm_fc4sc_inst_cov << "\n";
  out << "  },\n";
  out << "  \"P2_dist_100k\": {\n";
  out << "    \"draws_per_experiment\": " << N_P2 << ",\n";
  out << "    \"case1_0_2_split10_3_per2\": {\n";
  out << "      \"ieee_target_probs\": [0.277778, 0.277778, 0.277778, 0.166667],\n";
  out << "      \"unscaled_weighted_range\": {\n";
  out << "        \"counts\": " << vec_u64_json(p2_c1_unscaled.counts) << ",\n";
  out << "        \"tvd_vs_ieee\": " << p2_c1_unscaled.tvd_vs_ieee << ",\n";
  out << "        \"jsd_bits_vs_ieee\": " << p2_c1_unscaled.jsd_bits_vs_ieee << ",\n";
  out << "        \"chi2_vs_ieee\": " << p2_c1_unscaled.chi2_vs_ieee << ",\n";
  out << "        \"elapsed_ms\": " << p2_c1_unscaled.elapsed_ms << "\n";
  out << "      },\n";
  out << "      \"lcm_scaled_weighted_range\": {\n";
  out << "        \"counts\": " << vec_u64_json(p2_c1_lcm.counts) << ",\n";
  out << "        \"tvd_vs_ieee\": " << p2_c1_lcm.tvd_vs_ieee << ",\n";
  out << "        \"jsd_bits_vs_ieee\": " << p2_c1_lcm.jsd_bits_vs_ieee << ",\n";
  out << "        \"chi2_vs_ieee\": " << p2_c1_lcm.chi2_vs_ieee << ",\n";
  out << "        \"elapsed_ms\": " << p2_c1_lcm.elapsed_ms << "\n";
  out << "      },\n";
  out << "      \"tvd_unscaled_vs_lcm\": " << two_sample_tvd(p2_c1_unscaled.counts, p2_c1_lcm.counts, N_P2) << ",\n";
  out << "      \"naive_int_floor_per_elem\": {\n";
  out << "        \"counts\": " << vec_u64_json(p2_c1_int_floor.counts) << ",\n";
  out << "        \"tvd_vs_ieee\": " << p2_c1_int_floor.tvd_vs_ieee << ",\n";
  out << "        \"jsd_bits_vs_ieee\": " << p2_c1_int_floor.jsd_bits_vs_ieee << ",\n";
  out << "        \"chi2_vs_ieee\": " << p2_c1_int_floor.chi2_vs_ieee << "\n";
  out << "      },\n";
  out << "      \"constrained_x_ge_1_unscaled\": {\n";
  out << "        \"counts\": " << vec_u64_json(p2_c1_cond_unscaled.counts) << ",\n";
  out << "        \"tvd_vs_ieee\": " << p2_c1_cond_unscaled.tvd_vs_ieee << ",\n";
  out << "        \"chi2_vs_ieee\": " << p2_c1_cond_unscaled.chi2_vs_ieee << "\n";
  out << "      },\n";
  out << "      \"constrained_x_ge_1_lcm\": {\n";
  out << "        \"counts\": " << vec_u64_json(p2_c1_cond_lcm.counts) << ",\n";
  out << "        \"tvd_vs_ieee\": " << p2_c1_cond_lcm.tvd_vs_ieee << ",\n";
  out << "        \"chi2_vs_ieee\": " << p2_c1_cond_lcm.chi2_vs_ieee << "\n";
  out << "      }\n";
  out << "    },\n";
  out << "    \"case2_0_15_split5_16_per5\": {\n";
  out << "      \"unscaled_weighted_range\": {\n";
  out << "        \"counts\": " << vec_u64_json(p2_c2_unscaled.counts) << ",\n";
  out << "        \"tvd_vs_ieee\": " << p2_c2_unscaled.tvd_vs_ieee << ",\n";
  out << "        \"jsd_bits_vs_ieee\": " << p2_c2_unscaled.jsd_bits_vs_ieee << ",\n";
  out << "        \"chi2_vs_ieee\": " << p2_c2_unscaled.chi2_vs_ieee << ",\n";
  out << "        \"elapsed_ms\": " << p2_c2_unscaled.elapsed_ms << "\n";
  out << "      },\n";
  out << "      \"lcm_scaled_weighted_range\": {\n";
  out << "        \"counts\": " << vec_u64_json(p2_c2_lcm.counts) << ",\n";
  out << "        \"tvd_vs_ieee\": " << p2_c2_lcm.tvd_vs_ieee << ",\n";
  out << "        \"jsd_bits_vs_ieee\": " << p2_c2_lcm.jsd_bits_vs_ieee << ",\n";
  out << "        \"chi2_vs_ieee\": " << p2_c2_lcm.chi2_vs_ieee << ",\n";
  out << "        \"elapsed_ms\": " << p2_c2_lcm.elapsed_ms << "\n";
  out << "      },\n";
  out << "      \"tvd_unscaled_vs_lcm\": " << two_sample_tvd(p2_c2_unscaled.counts, p2_c2_lcm.counts, N_P2) << ",\n";
  out << "      \"naive_int_floor_per_elem\": {\n";
  out << "        \"counts\": " << vec_u64_json(p2_c2_int_floor.counts) << ",\n";
  out << "        \"tvd_vs_ieee\": " << p2_c2_int_floor.tvd_vs_ieee << ",\n";
  out << "        \"jsd_bits_vs_ieee\": " << p2_c2_int_floor.jsd_bits_vs_ieee << ",\n";
  out << "        \"chi2_vs_ieee\": " << p2_c2_int_floor.chi2_vs_ieee << "\n";
  out << "      },\n";
  out << "      \"constrained_x_ge_8_unscaled\": {\n";
  out << "        \"counts\": " << vec_u64_json(p2_c2_cond_unscaled.counts) << ",\n";
  out << "        \"tvd_vs_ieee\": " << p2_c2_cond_unscaled.tvd_vs_ieee << ",\n";
  out << "        \"chi2_vs_ieee\": " << p2_c2_cond_unscaled.chi2_vs_ieee << "\n";
  out << "      },\n";
  out << "      \"constrained_x_ge_8_lcm\": {\n";
  out << "        \"counts\": " << vec_u64_json(p2_c2_cond_lcm.counts) << ",\n";
  out << "        \"tvd_vs_ieee\": " << p2_c2_cond_lcm.tvd_vs_ieee << ",\n";
  out << "        \"chi2_vs_ieee\": " << p2_c2_cond_lcm.chi2_vs_ieee << "\n";
  out << "      }\n";
  out << "    }\n";
  out << "  },\n";
  out << "  \"P5_randc_scaling\": {\n";
  out << "    \"crave_native_randc_gt16_throws\": " << (native_gt16_throws ? "true" : "false") << ",\n";
  out << "    \"crave_native_randc_gt16_error\": \"" << native_gt16_err << "\",\n";
  out << "    \"crave_history_exclusion\": [\n";
  for (size_t i = 0; i < p5_hist.size(); ++i) {
    out << "      " << p5_row_json(p5_hist[i]) << (i + 1 < p5_hist.size() ? "," : "") << "\n";
  }
  out << "    ],\n";
  out << "    \"crave_native_randc\": [\n";
  for (size_t i = 0; i < p5_native.size(); ++i) {
    out << "      " << p5_row_json(p5_native[i]) << (i + 1 < p5_native.size() ? "," : "") << "\n";
  }
  out << "    ],\n";
  out << "    \"svcc_randc_feistel_pure\": [\n";
  for (size_t i = 0; i < p5_feistel_pure.size(); ++i) {
    out << "      " << p5_row_json(p5_feistel_pure[i]) << (i + 1 < p5_feistel_pure.size() ? "," : "") << "\n";
  }
  out << "    ],\n";
  out << "    \"svcc_randc_feistel_in_crave_item\": [\n";
  for (size_t i = 0; i < p5_feistel_crave.size(); ++i) {
    out << "      " << p5_row_json(p5_feistel_crave[i]) << (i + 1 < p5_feistel_crave.size() ? "," : "") << "\n";
  }
  out << "    ]\n";
  out << "  }\n";
  out << "}\n";
  out.close();

  std::cout << "Saved live_crave_results.json successfully." << std::endl;
  return 0;
}
