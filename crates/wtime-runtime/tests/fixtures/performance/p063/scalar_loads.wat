;; SPDX-License-Identifier: MIT
;; P063 one scalar memory load per export; addresses are supplied by the caller.
(module
  (memory (export "memory") 1 1)
  (data (i32.const 0) "\80\7f\01\80\02\03\04\80\01\80\80\01\02\80\01\02\03\04\05\06\07\08\00\00\80\3f\00\00\00\00\00\00\f0\3f")
  (func (export "i32_load") (param $address i32) (result i32)
    local.get $address
    i32.load
  )
  (func (export "i64_load") (param $address i32) (result i64)
    local.get $address
    i64.load
  )
  (func (export "f32_load") (param $address i32) (result f32)
    local.get $address
    f32.load
  )
  (func (export "f64_load") (param $address i32) (result f64)
    local.get $address
    f64.load
  )
  (func (export "i32_load8_s") (param $address i32) (result i32)
    local.get $address
    i32.load8_s
  )
  (func (export "i32_load8_u") (param $address i32) (result i32)
    local.get $address
    i32.load8_u
  )
  (func (export "i32_load16_s") (param $address i32) (result i32)
    local.get $address
    i32.load16_s
  )
  (func (export "i32_load16_u") (param $address i32) (result i32)
    local.get $address
    i32.load16_u
  )
  (func (export "i64_load8_s") (param $address i32) (result i64)
    local.get $address
    i64.load8_s
  )
  (func (export "i64_load8_u") (param $address i32) (result i64)
    local.get $address
    i64.load8_u
  )
  (func (export "i64_load16_s") (param $address i32) (result i64)
    local.get $address
    i64.load16_s
  )
  (func (export "i64_load16_u") (param $address i32) (result i64)
    local.get $address
    i64.load16_u
  )
  (func (export "i64_load32_s") (param $address i32) (result i64)
    local.get $address
    i64.load32_s
  )
  (func (export "i64_load32_u") (param $address i32) (result i64)
    local.get $address
    i64.load32_u
  )
)
