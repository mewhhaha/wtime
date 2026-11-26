// Fixed-width Wasm SIMD on the C backend's little-endian targets.
// Keep the scalar v128 ABI. Use vector types inside helpers so packed operations
// stay packed even when automatic loop and SLP vectorization are disabled.
#define WT_SIMD_TYPE(tag, scalar) \
    typedef scalar wt_##tag __attribute__((vector_size(16))); \
    static inline wt_##tag wt_as_##tag(wt_v128 v) { \
        wt_##tag r; memcpy(&r, &v, 16); return r; \
    } \
    static inline wt_v128 wt_from_##tag(wt_##tag v) { \
        wt_v128 r; memcpy(&r, &v, 16); return r; \
    }
WT_SIMD_TYPE(i8, int8_t)
WT_SIMD_TYPE(u8, uint8_t)
WT_SIMD_TYPE(i16, int16_t)
WT_SIMD_TYPE(u16, uint16_t)
WT_SIMD_TYPE(i32, int32_t)
WT_SIMD_TYPE(u32, uint32_t)
WT_SIMD_TYPE(i64, int64_t)
WT_SIMD_TYPE(u64, uint64_t)
WT_SIMD_TYPE(f32, float)
WT_SIMD_TYPE(f64, double)
#undef WT_SIMD_TYPE

static inline wt_v128 wt_simd_splat(unsigned bits, uint64_t x) {
    switch (bits) {
    case 8: return (wt_v128)(uint8_t)x * (((wt_v128)0x0101010101010101ULL << 64) | 0x0101010101010101ULL);
    case 16: return (wt_v128)(uint16_t)x * (((wt_v128)0x0001000100010001ULL << 64) | 0x0001000100010001ULL);
    case 32: return (wt_v128)(uint32_t)x * (((wt_v128)0x0000000100000001ULL << 64) | 0x0000000100000001ULL);
    default: return ((wt_v128)x << 64) | x;
    }
}

static inline wt_v128 wt_simd_replace(wt_v128 v, unsigned bits, unsigned lane, uint64_t x) {
    wt_v128 mask = UINT64_MAX >> (64 - bits);
    unsigned shift = bits * lane;
    return (v & ~(mask << shift)) | (((wt_v128)x & mask) << shift);
}

// Opcode suffixes match the decoded 0xfd-prefixed instruction. The emitter
// supplies the arity and result type, with no opcode dispatch at run time.
#define WT_UNARY(op) static inline wt_v128 wt_simd_##op(wt_v128 a)
#define WT_BINARY(op) static inline wt_v128 wt_simd_##op(wt_v128 a, wt_v128 b)
WT_BINARY(0e) { // i8x16.swizzle
    wt_u8 x = wt_as_u8(a), idx = wt_as_u8(b), r;
    for (unsigned i = 0; i < 16; ++i) r[i] = idx[i] < 16 ? x[idx[i]] : 0;
    return wt_from_u8(r);
}

#define WT_CMP(op, src, dst, pred) \
    WT_BINARY(op) { return wt_from_##dst((wt_##dst)(wt_as_##src(a) pred wt_as_##src(b))); }
#define WT_INT_CMP(eq, ne, lt_s, lt_u, gt_s, gt_u, le_s, le_u, ge_s, ge_u, bits) \
    WT_CMP(eq, u##bits, u##bits, ==) WT_CMP(ne, u##bits, u##bits, !=) \
    WT_CMP(lt_s, i##bits, u##bits, <) WT_CMP(lt_u, u##bits, u##bits, <) \
    WT_CMP(gt_s, i##bits, u##bits, >) WT_CMP(gt_u, u##bits, u##bits, >) \
    WT_CMP(le_s, i##bits, u##bits, <=) WT_CMP(le_u, u##bits, u##bits, <=) \
    WT_CMP(ge_s, i##bits, u##bits, >=) WT_CMP(ge_u, u##bits, u##bits, >=)
#define WT_SIGNED_CMP(eq, ne, lt, gt, le, ge, src, dst) \
    WT_CMP(eq, src, dst, ==) WT_CMP(ne, src, dst, !=) \
    WT_CMP(lt, src, dst, <) WT_CMP(gt, src, dst, >) \
    WT_CMP(le, src, dst, <=) WT_CMP(ge, src, dst, >=)
WT_INT_CMP(23, 24, 25, 26, 27, 28, 29, 2a, 2b, 2c, 8)
WT_INT_CMP(2d, 2e, 2f, 30, 31, 32, 33, 34, 35, 36, 16)
WT_INT_CMP(37, 38, 39, 3a, 3b, 3c, 3d, 3e, 3f, 40, 32)
WT_SIGNED_CMP(41, 42, 43, 44, 45, 46, f32, u32)
WT_SIGNED_CMP(47, 48, 49, 4a, 4b, 4c, f64, u64)
WT_SIGNED_CMP(d6, d7, d8, d9, da, db, i64, u64)
#undef WT_SIGNED_CMP
#undef WT_INT_CMP
#undef WT_CMP

WT_UNARY(4d) { return ~a; }
WT_BINARY(4e) { return a & b; }
WT_BINARY(4f) { return a & ~b; }
WT_BINARY(50) { return a | b; }
WT_BINARY(51) { return a ^ b; }
static inline wt_v128 wt_simd_52(wt_v128 a, wt_v128 b, wt_v128 mask) {
    return (a & mask) | (b & ~mask);
}
static inline int32_t wt_simd_53(wt_v128 a) { return a != 0; }

#define WT_INT_UNARY(abs_op, neg, all, mask, bits, lanes) \
    WT_UNARY(abs_op) { \
        wt_u##bits x = wt_as_u##bits(a); \
        wt_u##bits sign = (wt_u##bits)(wt_as_i##bits(a) < 0); \
        return wt_from_u##bits((x ^ sign) - sign); \
    } \
    WT_UNARY(neg) { return wt_from_u##bits(-wt_as_u##bits(a)); } \
    static inline int32_t wt_simd_##all(wt_v128 a) { \
        return wt_from_u##bits((wt_u##bits)(wt_as_u##bits(a) == 0)) == 0; \
    } \
    static inline int32_t wt_simd_##mask(wt_v128 a) { \
        wt_u##bits x = wt_as_u##bits(a); \
        uint32_t r = 0; \
        for (unsigned i = 0; i < lanes; ++i) r |= (uint32_t)(x[i] >> (bits - 1)) << i; \
        return (int32_t)r; \
    }
WT_INT_UNARY(60, 61, 63, 64, 8, 16)
WT_INT_UNARY(80, 81, 83, 84, 16, 8)
WT_INT_UNARY(a0, a1, a3, a4, 32, 4)
WT_INT_UNARY(c0, c1, c3, c4, 64, 2)
#undef WT_INT_UNARY

WT_UNARY(62) {
    wt_u8 x = wt_as_u8(a), r;
    for (unsigned i = 0; i < 16; ++i) r[i] = __builtin_popcount(x[i]);
    return wt_from_u8(r);
}

#define WT_SHIFT(op, tag, sign, bits, shift) \
    static inline wt_v128 wt_simd_##op(wt_v128 a, int32_t n) { \
        return wt_from_##tag((wt_##tag)(wt_as_##sign(a) shift ((uint32_t)n & (bits - 1)))); \
    }
#define WT_SHIFTS(shl, shr_s, shr_u, bits) \
    WT_SHIFT(shl, u##bits, u##bits, bits, <<) \
    WT_SHIFT(shr_s, u##bits, i##bits, bits, >>) \
    WT_SHIFT(shr_u, u##bits, u##bits, bits, >>)
WT_SHIFTS(6b, 6c, 6d, 8)
WT_SHIFTS(8b, 8c, 8d, 16)
WT_SHIFTS(ab, ac, ad, 32)
WT_SHIFTS(cb, cc, cd, 64)
#undef WT_SHIFTS
#undef WT_SHIFT

#define WT_ARITH(op, tag, operator) \
    WT_BINARY(op) { return wt_from_##tag(wt_as_##tag(a) operator wt_as_##tag(b)); }
WT_ARITH(6e, u8, +)
WT_ARITH(71, u8, -)
WT_ARITH(8e, u16, +)
WT_ARITH(91, u16, -)
WT_ARITH(95, u16, *)
WT_ARITH(ae, u32, +)
WT_ARITH(b1, u32, -)
WT_ARITH(b5, u32, *)
WT_ARITH(ce, u64, +)
WT_ARITH(d1, u64, -)
WT_ARITH(d5, u64, *)
WT_ARITH(e4, f32, +)
WT_ARITH(e5, f32, -)
WT_ARITH(e6, f32, *)
WT_ARITH(e7, f32, /)
WT_ARITH(f0, f64, +)
WT_ARITH(f1, f64, -)
WT_ARITH(f2, f64, *)
WT_ARITH(f3, f64, /)
#undef WT_ARITH

// Each lane is independent, with widening before any arithmetic that
// saturates or needs an intermediate wider than the source.
#define WT_MAP(op, src, dst, lanes, expr) \
    WT_UNARY(op) { \
        wt_##src x = wt_as_##src(a); wt_##dst r = {0}; \
        for (unsigned i = 0; i < lanes; ++i) r[i] = (expr); \
        return wt_from_##dst(r); \
    }
#define WT_ZIP(op, src, dst, lanes, expr) \
    WT_BINARY(op) { \
        wt_##src x = wt_as_##src(a), y = wt_as_##src(b); wt_##dst r; \
        for (unsigned i = 0; i < lanes; ++i) r[i] = (expr); \
        return wt_from_##dst(r); \
    }
#define WT_CLAMP(x, lo, hi) ((x) < (lo) ? (lo) : (x) > (hi) ? (hi) : (x))
#define WT_NARROW(op, src, dst, lanes, lo, hi) \
    WT_BINARY(op) { \
        wt_##src x = wt_as_##src(a), y = wt_as_##src(b); wt_##dst r; \
        for (unsigned i = 0; i < lanes; ++i) { \
            r[i] = WT_CLAMP(x[i], lo, hi); \
            r[i + lanes] = WT_CLAMP(y[i], lo, hi); \
        } \
        return wt_from_##dst(r); \
    }
WT_NARROW(65, i16, i8, 8, -128, 127)
WT_NARROW(66, i16, u8, 8, 0, 255)
WT_NARROW(85, i32, i16, 4, -32768, 32767)
WT_NARROW(86, i32, u16, 4, 0, 65535)
#undef WT_NARROW

#define WT_SAT(op, tag, lanes, operator, lo, hi) \
    WT_ZIP(op, tag, tag, lanes, WT_CLAMP((int32_t)x[i] operator (int32_t)y[i], lo, hi))
WT_SAT(6f, i8, 16, +, -128, 127)
WT_SAT(70, u8, 16, +, 0, 255)
WT_SAT(72, i8, 16, -, -128, 127)
WT_SAT(73, u8, 16, -, 0, 255)
WT_SAT(8f, i16, 8, +, -32768, 32767)
WT_SAT(90, u16, 8, +, 0, 65535)
WT_SAT(92, i16, 8, -, -32768, 32767)
WT_SAT(93, u16, 8, -, 0, 65535)
#undef WT_SAT
WT_ZIP(82, i16, i16, 8, WT_CLAMP(((int32_t)x[i] * y[i] + 16384) >> 15, -32768, 32767))
#undef WT_CLAMP

#define WT_MINMAX(min, max, tag, lanes) \
    WT_ZIP(min, tag, tag, lanes, x[i] < y[i] ? x[i] : y[i]) \
    WT_ZIP(max, tag, tag, lanes, x[i] > y[i] ? x[i] : y[i])
WT_MINMAX(76, 78, i8, 16)
WT_MINMAX(77, 79, u8, 16)
WT_MINMAX(96, 98, i16, 8)
WT_MINMAX(97, 99, u16, 8)
WT_MINMAX(b6, b8, i32, 4)
WT_MINMAX(b7, b9, u32, 4)
#undef WT_MINMAX
WT_ZIP(7b, u8, u8, 16, ((uint32_t)x[i] + y[i] + 1u) >> 1)
WT_ZIP(9b, u16, u16, 8, ((uint32_t)x[i] + y[i] + 1u) >> 1)

WT_MAP(7c, i8, i16, 8, x[2 * i] + x[2 * i + 1])
WT_MAP(7d, u8, u16, 8, x[2 * i] + x[2 * i + 1])
WT_MAP(7e, i16, i32, 4, x[2 * i] + x[2 * i + 1])
WT_MAP(7f, u16, u32, 4, x[2 * i] + x[2 * i + 1])
// Both products fit i32. Their sum wraps in u32.
WT_ZIP(ba, i16, u32, 4, (uint32_t)((int32_t)x[2 * i] * y[2 * i])
                        + (uint32_t)((int32_t)x[2 * i + 1] * y[2 * i + 1]))

#define WT_EXTEND(low, high, src, dst, lanes) \
    WT_MAP(low, src, dst, lanes, x[i]) \
    WT_MAP(high, src, dst, lanes, x[i + lanes])
WT_EXTEND(87, 88, i8, i16, 8)
WT_EXTEND(89, 8a, u8, u16, 8)
WT_EXTEND(a7, a8, i16, i32, 4)
WT_EXTEND(a9, aa, u16, u32, 4)
WT_EXTEND(c7, c8, i32, i64, 2)
WT_EXTEND(c9, ca, u32, u64, 2)
#undef WT_EXTEND
#define WT_EXTMUL(low, high, src, dst, lanes, wide) \
    WT_ZIP(low, src, dst, lanes, (wide)x[i] * y[i]) \
    WT_ZIP(high, src, dst, lanes, (wide)x[i + lanes] * y[i + lanes])
WT_EXTMUL(9c, 9d, i8, i16, 8, int32_t)
WT_EXTMUL(9e, 9f, u8, u16, 8, uint32_t)
WT_EXTMUL(bc, bd, i16, i32, 4, int32_t)
WT_EXTMUL(be, bf, u16, u32, 4, uint32_t)
WT_EXTMUL(dc, dd, i32, i64, 2, int64_t)
WT_EXTMUL(de, df, u32, u64, 2, uint64_t)
#undef WT_EXTMUL

#define WT_FLOAT_BITS(abs_op, neg, tag, sign) \
    WT_UNARY(abs_op) { return wt_from_##tag(wt_as_##tag(a) & ~(sign)); } \
    WT_UNARY(neg) { return wt_from_##tag(wt_as_##tag(a) ^ (sign)); }
WT_FLOAT_BITS(e0, e1, u32, 0x80000000u)
WT_FLOAT_BITS(ec, ed, u64, 0x8000000000000000ULL)
#undef WT_FLOAT_BITS
WT_MAP(67, f32, f32, 4, __builtin_ceilf(x[i]))
WT_MAP(68, f32, f32, 4, __builtin_floorf(x[i]))
WT_MAP(69, f32, f32, 4, __builtin_truncf(x[i]))
WT_MAP(6a, f32, f32, 4, __builtin_roundevenf(x[i]))
WT_MAP(e3, f32, f32, 4, __builtin_sqrtf(x[i]))
WT_MAP(74, f64, f64, 2, __builtin_ceil(x[i]))
WT_MAP(75, f64, f64, 2, __builtin_floor(x[i]))
WT_MAP(7a, f64, f64, 2, __builtin_trunc(x[i]))
WT_MAP(94, f64, f64, 2, __builtin_roundeven(x[i]))
WT_MAP(ef, f64, f64, 2, __builtin_sqrt(x[i]))
#define WT_FLOAT_MINMAX(min, max, pmin, pmax, tag, lanes) \
    WT_ZIP(min, tag, tag, lanes, wt_##tag##_min(x[i], y[i])) \
    WT_ZIP(max, tag, tag, lanes, wt_##tag##_max(x[i], y[i])) \
    WT_ZIP(pmin, tag, tag, lanes, y[i] < x[i] ? y[i] : x[i]) \
    WT_ZIP(pmax, tag, tag, lanes, x[i] < y[i] ? y[i] : x[i])
WT_FLOAT_MINMAX(e8, e9, ea, eb, f32, 4)
WT_FLOAT_MINMAX(f4, f5, f6, f7, f64, 2)
#undef WT_FLOAT_MINMAX

WT_MAP(5e, f64, f32, 2, (float)x[i])  // demote_f64x2_zero
WT_MAP(5f, f32, f64, 2, (double)x[i]) // promote_low_f32x4
WT_MAP(f8, f32, i32, 4, wt_i32_trunc_sat_f32_s(x[i]))
WT_MAP(f9, f32, i32, 4, wt_i32_trunc_sat_f32_u(x[i]))
WT_UNARY(fa) { return wt_from_f32(__builtin_convertvector(wt_as_i32(a), wt_f32)); }
WT_UNARY(fb) { // f32x4.convert_i32x4_u: one round-to-nearest-even conversion per lane
    return wt_from_f32(__builtin_convertvector(wt_as_u32(a), wt_f32));
}
WT_MAP(fc, f64, i32, 2, wt_i32_trunc_sat_f64_s(x[i]))
WT_MAP(fd, f64, i32, 2, wt_i32_trunc_sat_f64_u(x[i]))
WT_MAP(fe, i32, f64, 2, (double)x[i])
WT_MAP(ff, u32, f64, 2, (double)x[i])
#undef WT_MAP
#undef WT_ZIP
#undef WT_UNARY
#undef WT_BINARY
