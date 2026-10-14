;; SPDX-License-Identifier: MIT
;; P062 scalar floating-point opcode and conversion corpus.
(module
  (func (export "f32_eq") (param $p0 f32) (param $p1 f32) (result i32)
    local.get $p0
    local.get $p1
    f32.eq
  )
  (func (export "f32_ne") (param $p0 f32) (param $p1 f32) (result i32)
    local.get $p0
    local.get $p1
    f32.ne
  )
  (func (export "f32_lt") (param $p0 f32) (param $p1 f32) (result i32)
    local.get $p0
    local.get $p1
    f32.lt
  )
  (func (export "f32_gt") (param $p0 f32) (param $p1 f32) (result i32)
    local.get $p0
    local.get $p1
    f32.gt
  )
  (func (export "f32_le") (param $p0 f32) (param $p1 f32) (result i32)
    local.get $p0
    local.get $p1
    f32.le
  )
  (func (export "f32_ge") (param $p0 f32) (param $p1 f32) (result i32)
    local.get $p0
    local.get $p1
    f32.ge
  )
  (func (export "f64_eq") (param $p0 f64) (param $p1 f64) (result i32)
    local.get $p0
    local.get $p1
    f64.eq
  )
  (func (export "f64_ne") (param $p0 f64) (param $p1 f64) (result i32)
    local.get $p0
    local.get $p1
    f64.ne
  )
  (func (export "f64_lt") (param $p0 f64) (param $p1 f64) (result i32)
    local.get $p0
    local.get $p1
    f64.lt
  )
  (func (export "f64_gt") (param $p0 f64) (param $p1 f64) (result i32)
    local.get $p0
    local.get $p1
    f64.gt
  )
  (func (export "f64_le") (param $p0 f64) (param $p1 f64) (result i32)
    local.get $p0
    local.get $p1
    f64.le
  )
  (func (export "f64_ge") (param $p0 f64) (param $p1 f64) (result i32)
    local.get $p0
    local.get $p1
    f64.ge
  )
  (func (export "f32_abs") (param $p0 f32) (result f32)
    local.get $p0
    f32.abs
  )
  (func (export "f32_neg") (param $p0 f32) (result f32)
    local.get $p0
    f32.neg
  )
  (func (export "f32_ceil") (param $p0 f32) (result f32)
    local.get $p0
    f32.ceil
  )
  (func (export "f32_floor") (param $p0 f32) (result f32)
    local.get $p0
    f32.floor
  )
  (func (export "f32_trunc") (param $p0 f32) (result f32)
    local.get $p0
    f32.trunc
  )
  (func (export "f32_nearest") (param $p0 f32) (result f32)
    local.get $p0
    f32.nearest
  )
  (func (export "f32_sqrt") (param $p0 f32) (result f32)
    local.get $p0
    f32.sqrt
  )
  (func (export "f32_add") (param $p0 f32) (param $p1 f32) (result f32)
    local.get $p0
    local.get $p1
    f32.add
  )
  (func (export "f32_sub") (param $p0 f32) (param $p1 f32) (result f32)
    local.get $p0
    local.get $p1
    f32.sub
  )
  (func (export "f32_mul") (param $p0 f32) (param $p1 f32) (result f32)
    local.get $p0
    local.get $p1
    f32.mul
  )
  (func (export "f32_div") (param $p0 f32) (param $p1 f32) (result f32)
    local.get $p0
    local.get $p1
    f32.div
  )
  (func (export "f32_min") (param $p0 f32) (param $p1 f32) (result f32)
    local.get $p0
    local.get $p1
    f32.min
  )
  (func (export "f32_max") (param $p0 f32) (param $p1 f32) (result f32)
    local.get $p0
    local.get $p1
    f32.max
  )
  (func (export "f32_copysign") (param $p0 f32) (param $p1 f32) (result f32)
    local.get $p0
    local.get $p1
    f32.copysign
  )
  (func (export "f64_abs") (param $p0 f64) (result f64)
    local.get $p0
    f64.abs
  )
  (func (export "f64_neg") (param $p0 f64) (result f64)
    local.get $p0
    f64.neg
  )
  (func (export "f64_ceil") (param $p0 f64) (result f64)
    local.get $p0
    f64.ceil
  )
  (func (export "f64_floor") (param $p0 f64) (result f64)
    local.get $p0
    f64.floor
  )
  (func (export "f64_trunc") (param $p0 f64) (result f64)
    local.get $p0
    f64.trunc
  )
  (func (export "f64_nearest") (param $p0 f64) (result f64)
    local.get $p0
    f64.nearest
  )
  (func (export "f64_sqrt") (param $p0 f64) (result f64)
    local.get $p0
    f64.sqrt
  )
  (func (export "f64_add") (param $p0 f64) (param $p1 f64) (result f64)
    local.get $p0
    local.get $p1
    f64.add
  )
  (func (export "f64_sub") (param $p0 f64) (param $p1 f64) (result f64)
    local.get $p0
    local.get $p1
    f64.sub
  )
  (func (export "f64_mul") (param $p0 f64) (param $p1 f64) (result f64)
    local.get $p0
    local.get $p1
    f64.mul
  )
  (func (export "f64_div") (param $p0 f64) (param $p1 f64) (result f64)
    local.get $p0
    local.get $p1
    f64.div
  )
  (func (export "f64_min") (param $p0 f64) (param $p1 f64) (result f64)
    local.get $p0
    local.get $p1
    f64.min
  )
  (func (export "f64_max") (param $p0 f64) (param $p1 f64) (result f64)
    local.get $p0
    local.get $p1
    f64.max
  )
  (func (export "f64_copysign") (param $p0 f64) (param $p1 f64) (result f64)
    local.get $p0
    local.get $p1
    f64.copysign
  )
  (func (export "i32_trunc_f32_s") (param $p0 f32) (result i32)
    local.get $p0
    i32.trunc_f32_s
  )
  (func (export "i32_trunc_f32_u") (param $p0 f32) (result i32)
    local.get $p0
    i32.trunc_f32_u
  )
  (func (export "i32_trunc_f64_s") (param $p0 f64) (result i32)
    local.get $p0
    i32.trunc_f64_s
  )
  (func (export "i32_trunc_f64_u") (param $p0 f64) (result i32)
    local.get $p0
    i32.trunc_f64_u
  )
  (func (export "i64_trunc_f32_s") (param $p0 f32) (result i64)
    local.get $p0
    i64.trunc_f32_s
  )
  (func (export "i64_trunc_f32_u") (param $p0 f32) (result i64)
    local.get $p0
    i64.trunc_f32_u
  )
  (func (export "i64_trunc_f64_s") (param $p0 f64) (result i64)
    local.get $p0
    i64.trunc_f64_s
  )
  (func (export "i64_trunc_f64_u") (param $p0 f64) (result i64)
    local.get $p0
    i64.trunc_f64_u
  )
  (func (export "f32_convert_i32_s") (param $p0 i32) (result f32)
    local.get $p0
    f32.convert_i32_s
  )
  (func (export "f32_convert_i32_u") (param $p0 i32) (result f32)
    local.get $p0
    f32.convert_i32_u
  )
  (func (export "f32_convert_i64_s") (param $p0 i64) (result f32)
    local.get $p0
    f32.convert_i64_s
  )
  (func (export "f32_convert_i64_u") (param $p0 i64) (result f32)
    local.get $p0
    f32.convert_i64_u
  )
  (func (export "f64_convert_i32_s") (param $p0 i32) (result f64)
    local.get $p0
    f64.convert_i32_s
  )
  (func (export "f64_convert_i32_u") (param $p0 i32) (result f64)
    local.get $p0
    f64.convert_i32_u
  )
  (func (export "f64_convert_i64_s") (param $p0 i64) (result f64)
    local.get $p0
    f64.convert_i64_s
  )
  (func (export "f64_convert_i64_u") (param $p0 i64) (result f64)
    local.get $p0
    f64.convert_i64_u
  )
  (func (export "f32_demote_f64") (param $p0 f64) (result f32)
    local.get $p0
    f32.demote_f64
  )
  (func (export "f64_promote_f32") (param $p0 f32) (result f64)
    local.get $p0
    f64.promote_f32
  )
  (func (export "i32_reinterpret_f32") (param $p0 f32) (result i32)
    local.get $p0
    i32.reinterpret_f32
  )
  (func (export "i64_reinterpret_f64") (param $p0 f64) (result i64)
    local.get $p0
    i64.reinterpret_f64
  )
  (func (export "f32_reinterpret_i32") (param $p0 i32) (result f32)
    local.get $p0
    f32.reinterpret_i32
  )
  (func (export "f64_reinterpret_i64") (param $p0 i64) (result f64)
    local.get $p0
    f64.reinterpret_i64
  )
  (func (export "i32_trunc_sat_f32_s") (param $p0 f32) (result i32)
    local.get $p0
    i32.trunc_sat_f32_s
  )
  (func (export "i32_trunc_sat_f32_u") (param $p0 f32) (result i32)
    local.get $p0
    i32.trunc_sat_f32_u
  )
  (func (export "i32_trunc_sat_f64_s") (param $p0 f64) (result i32)
    local.get $p0
    i32.trunc_sat_f64_s
  )
  (func (export "i32_trunc_sat_f64_u") (param $p0 f64) (result i32)
    local.get $p0
    i32.trunc_sat_f64_u
  )
  (func (export "i64_trunc_sat_f32_s") (param $p0 f32) (result i64)
    local.get $p0
    i64.trunc_sat_f32_s
  )
  (func (export "i64_trunc_sat_f32_u") (param $p0 f32) (result i64)
    local.get $p0
    i64.trunc_sat_f32_u
  )
  (func (export "i64_trunc_sat_f64_s") (param $p0 f64) (result i64)
    local.get $p0
    i64.trunc_sat_f64_s
  )
  (func (export "i64_trunc_sat_f64_u") (param $p0 f64) (result i64)
    local.get $p0
    i64.trunc_sat_f64_u
  )
)
