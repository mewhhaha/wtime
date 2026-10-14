;; SPDX-License-Identifier: Apache-2.0 OR MIT
(module
  (func (export "run") (param $x i32) (param $condition i32) (result i32) (local $temporary i32)
    local.get $x
    i32.const 2147483647
    i32.add
    local.tee $temporary
    i32.const 1
    i32.xor
    local.set $temporary
    local.get $temporary
    i32.const 7
    local.get $condition
    select
    i32.const 99
    drop))
