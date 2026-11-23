;; SPDX-License-Identifier: CC0-1.0
;; P113 draft fixture. Encoded and validated by the integrator before installation.
(module
  (type $step (func (param i32) (result i32)))
  (type $cont (cont $step))
  (tag $yield (type $step))
  (memory (export "memory") 1)
  (func (export "run") (type $step) (local $tmp i32)
    local.get 0
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
    (local.set $tmp (suspend $yield))
    i32.const 0
    local.get $tmp
    i32.store
    local.get $tmp
  )
  (func (export "resume") (param $continuation (ref $cont))
                            (param $value i32) (result i32)
    local.get $value
    local.get $continuation
    resume $cont
  )
)

