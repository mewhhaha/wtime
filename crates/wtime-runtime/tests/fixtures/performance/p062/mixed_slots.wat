;; SPDX-License-Identifier: MIT
;; P062 mixed numeric parameter/local slots with bit-preserving copies.
(module
  (func (export "run") (param $pi32 i32) (param $pi64 i64) (param $pf32 f32) (param $pf64 f64)
    (result i32 i64 f32 f64)
    (local $li32 i32) (local $li64 i64) (local $lf32 f32) (local $lf64 f64)
    local.get $pi32
    local.tee $li32
    drop
    local.get $pi64
    local.set $li64
    local.get $pf32
    local.set $lf32
    local.get $pf64
    local.set $lf64
    local.get $li32
    local.get $li64
    local.get $lf32
    local.get $lf64
  )
)
