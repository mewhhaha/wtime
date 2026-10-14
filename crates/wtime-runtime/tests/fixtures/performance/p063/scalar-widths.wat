;; Original P043 fixture; dual MIT OR Apache-2.0. Source-only, not encoded.
(module
  (memory (export "memory") 1 1)
  ;; Distinct unaligned little-endian patterns cover every load width/sign.
  (data (i32.const 0) "\80\7f\01\80\02\03\04\80\01\80\80\01\02\80\01\02\03\04\05\06\07\08\00\00\80\3f\00\00\00\00\00\00\f0\3f")
  (func (export "run") (result i32 i32 i32 i32 i32 i64 i64 i64 i64 i64 i64 i64 f32 f64)
    (i32.load8_s (i32.const 0))
    (i32.load8_u (i32.const 0))
    (i32.load16_s (i32.const 1))
    (i32.load16_u (i32.const 1))
    (i32.load (i32.const 3))
    (i64.load8_s (i32.const 7))
    (i64.load8_u (i32.const 7))
    (i64.load16_s (i32.const 8))
    (i64.load16_u (i32.const 8))
    (i64.load32_s (i32.const 10))
    (i64.load32_u (i32.const 10))
    (i64.load (i32.const 14))
    (f32.load (i32.const 22))
    (f64.load (i32.const 26)))

  ;; Nine stores are each exactly address, local.get, store in source order.
  (func (export "store_kinds") (param $v32 i32) (param $v64 i64) (param $vf32 f32) (param $vf64 f64)
    i32.const 65
    local.get $v32
    i32.store8
    i32.const 67
    local.get $v32
    i32.store16
    i32.const 71
    local.get $v32
    i32.store
    i32.const 79
    local.get $v64
    i64.store8
    i32.const 81
    local.get $v64
    i64.store16
    i32.const 85
    local.get $v64
    i64.store32
    i32.const 91
    local.get $v64
    i64.store
    i32.const 103
    local.get $vf32
    f32.store
    i32.const 109
    local.get $vf64
    f64.store)
  (func (export "float_raw_bits") (result i32 i64)
    (i32.reinterpret_f32 (f32.load (i32.const 22)))
    (i64.reinterpret_f64 (f64.load (i32.const 26))))
  ;; An atomic guest access promotes the unshared backend lazily.
  ;; The first seeded byte is 0x80, so unsigned load8 returns 128.
  (func (export "atomic_promote") (result i32)
    (i32.atomic.load8_u (i32.const 0)))
)
