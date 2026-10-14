;; Original CC0 P028 fixture: integer arithmetic, masked shifts, comparisons,
;; unary bit-count operations, and fuel-visible prefix stores.
(module
  (memory (export "memory") 1)

  (func (export "wrap32") (result i32 i32 i32)
    (i32.add (i32.const 2147483647) (i32.const 1))
    (i32.sub (i32.const -2147483648) (i32.const 1))
    (i32.mul (i32.const 1073741824) (i32.const 4)))
  (func (export "wrap64") (result i64 i64 i64)
    (i64.add (i64.const 9223372036854775807) (i64.const 1))
    (i64.sub (i64.const -9223372036854775808) (i64.const 1))
    (i64.mul (i64.const 4611686018427387904) (i64.const 4)))

  (func (export "bitwise32") (result i32 i32 i32)
    (i32.and (i32.const -252702961) (i32.const 267452160))
    (i32.or (i32.const -252702961) (i32.const 267452160))
    (i32.xor (i32.const -252702961) (i32.const 267452160)))
  (func (export "bitwise64") (result i64 i64 i64)
    (i64.and (i64.const -1085350949069787121) (i64.const 1148435428461776880))
    (i64.or (i64.const -1085350949069787121) (i64.const 1148435428461776880))
    (i64.xor (i64.const -1085350949069787121) (i64.const 1148435428461776880)))

  (func (export "shifts32") (result i32 i32 i32 i32)
    (i32.shl (i32.const 1) (i32.const 32))
    (i32.shl (i32.const 1) (i32.const 33))
    (i32.shr_s (i32.const -16) (i32.const 33))
    (i32.shr_u (i32.const -2147483648) (i32.const 32)))
  (func (export "shifts64") (result i64 i64 i64 i64)
    (i64.shl (i64.const 1) (i64.const 64))
    (i64.shl (i64.const 1) (i64.const 65))
    (i64.shr_s (i64.const -16) (i64.const 65))
    (i64.shr_u (i64.const -9223372036854775808) (i64.const 64)))

  (func (export "compare32") (result i32 i32 i32 i32)
    (i32.lt_s (i32.const -1) (i32.const 1))
    (i32.lt_u (i32.const -1) (i32.const 1))
    (i32.gt_s (i32.const -1) (i32.const 1))
    (i32.gt_u (i32.const -1) (i32.const 1)))
  (func (export "compare64") (result i32 i32 i32 i32)
    (i64.lt_s (i64.const -1) (i64.const 1))
    (i64.lt_u (i64.const -1) (i64.const 1))
    (i64.gt_s (i64.const -1) (i64.const 1))
    (i64.gt_u (i64.const -1) (i64.const 1)))

  (func (export "unary32") (result i32 i32 i32)
    (i32.clz (i32.const 15728640))
    (i32.ctz (i32.const 16777216))
    (i32.popcnt (i32.const -268435455)))
  (func (export "unary64") (result i64 i64 i64)
    (i64.clz (i64.const 2147483648))
    (i64.ctz (i64.const 4294967296))
    (i64.popcnt (i64.const -9223372036854775807)))

  ;; Each prefix group is five source instructions: const/get/get/op/store.
  ;; With exact per-source-instruction fuel, 5/10/15 fuel admits each store;
  ;; 16 fuel admits the final constant and returns normally.
  (func (export "fuel_prefix") (param $a i32) (param $b i32) (result i32)
    (i32.store (i32.const 0) (i32.add (local.get $a) (local.get $b)))
    (i32.store (i32.const 4) (i32.sub (local.get $a) (local.get $b)))
    (i32.store (i32.const 8) (i32.mul (local.get $a) (local.get $b)))
    (i32.const 42)))
