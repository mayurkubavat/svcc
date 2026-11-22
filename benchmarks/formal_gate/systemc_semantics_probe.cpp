#include <systemc>
#include <iostream>
#include <typeinfo>
#include <type_traits>
using namespace sc_dt;
#define SHOW(e) std::cout << #e << " = " << (e) << "\n"
template<class T> const char* cls(const T&) {
  if (std::is_same<T, sc_unsigned>::value) return "sc_unsigned";
  if (std::is_same<T, sc_signed>::value) return "sc_signed";
  if (std::is_same<T, uint64>::value) return "uint64";
  if (std::is_same<T, int64>::value) return "int64";
  if (std::is_same<T, int>::value) return "int";
  if (std::is_same<T, unsigned>::value) return "unsigned";
  if (std::is_same<T, bool>::value) return "bool";
  return typeid(T).name();
}
#define TY(e) std::cout << #e << " : " << cls(e) << " len=" << (e).length() << " val=" << (e) << "\n"
#define TYB(e) std::cout << #e << " : " << cls(e) << " val=" << (e) << "\n"
int sc_main(int, char**) {
  sc_biguint<72> a = 5, b = 9; sc_bigint<72> s = -3;
  sc_uint<8> u = 200, v = 100; sc_int<8> i = -5; sc_uint<64> u64 = ~0ULL;
  TY(a + b); TY(a - b); TY(a * b); TY(a & b); TY(a | s); TY(~a); TY(-a); TY(a << 3); TY(a >> 1);
  TY(a + 1); TY(a + 1U); TY(a + 1ULL); TY(a - 1ULL); TY(a + u); TY(a + i); TY(s + a); TY(a / b); TY(a % b);
  TY(a ^ b); TY(~s); TY(a + uint64_t(7)); TY(a << uint64_t(70)); TY(s >> 1); TY(s << 2);
  TYB(a < b); TYB(a < s); TYB(s < a); TYB(a == 5); TYB(a > -1); TYB(a - b < 0);
  TYB(u + v); TYB(u * v); TYB(-u); TYB(~u); TYB(i * 3); TYB(u + i); TYB(i >> 1); TYB(u << 60); TYB(u64 + u64);
  TYB(u < 0); TYB(i < 0); TYB(u < i); TYB(i < u); TYB(u == 200); TYB(i == u);
  std::cout << "concat: " << typeid((u, v)).name() << "\n";
  sc_uint<16> c16 = (u, v); SHOW(c16);
  sc_biguint<128> c128 = (u64, u64); SHOW(c128.to_string(SC_HEX));
  sc_uint<9> c9 = (sc_uint<1>(true), u); SHOW(c9);
  sc_uint<8> w = 0; w[3] = 2; SHOW(w); w[2] = true; SHOW(w); w.range(7,4) = 0x1ff; SHOW(w);
  sc_uint<8> t8 = sc_uint<8>(a); SHOW(t8); sc_uint<8> t9(s); SHOW(t9);
  sc_biguint<72> fromneg = sc_biguint<72>(s); SHOW(fromneg.to_string(SC_HEX));
  sc_biguint<72> nb = sc_biguint<72>(~a); SHOW(nb.to_string(SC_HEX));
  sc_biguint<72> nn = sc_biguint<72>(-a); SHOW(nn.to_string(SC_HEX));
  SHOW((~a == nb)); SHOW((~a).to_string(SC_HEX));
  SHOW(a.range(71, 60).to_uint64()); TY(a.range(40, 3)); 
  sc_biguint<72> big = (sc_biguint<72>(1) << 71) | 3; SHOW(big.to_uint64()); SHOW(sc_uint<8>(big.range(71,64).to_uint64()));
  SHOW(sc_int<8>(200)); SHOW(sc_uint<8>(-1)); SHOW(sc_int<4>(i)); SHOW(int(sc_uint<64>(0xffffffff00000001ULL)));
  SHOW(u.xor_reduce()); SHOW(a.and_reduce()); SHOW(bool(u[3])); SHOW(u.range(7,4)); SHOW(u.range(7,4) + 1);
  TYB(u.range(7,4) + 1); TYB(u[3] + 1); TYB(true + u); TYB(u >> v); TYB(sc_uint<8>(3) - sc_uint<8>(5));
  TYB(i / 2); TYB(i % 3); TYB(-i); TYB(i + 1U); TYB(u + 1); TYB(u + 1U); TYB(u - 201);
  SHOW((a << 0)); SHOW(sc_biguint<72>(a << uint64_t(72)).to_string(SC_HEX));
  SHOW(sc_bigint<72>(s >> 100)); SHOW(sc_biguint<72>(a >> 100));
  return 0;
}
