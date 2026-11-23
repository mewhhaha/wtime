;; SPDX-License-Identifier: CC0-1.0
;; P113 draft fixture. Encoded and validated by the integrator before installation.
(module
  (type $box (struct (field i32)))
  (memory (export "memory") 1)
  (func (export "run") (param $n i32) (result i32) (local $tmp i32)
    local.get $n
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    i32.const 1
    i32.add
    local.set $tmp
    local.get $tmp
    struct.new $box
    struct.get $box 0
    local.set $tmp
    i32.const 0
    local.get $tmp
    i32.store
    local.get $tmp
  )
)

