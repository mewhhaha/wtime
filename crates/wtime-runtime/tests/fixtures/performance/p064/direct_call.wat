;; P064 direct guest call with caller memory effect before the call. MIT OR Apache-2.0.
(module
  (memory (export "memory") 1 1)
  (func $leaf (export "leaf") (param $x i32) (result i32)
    local.get $x
    i32.const 2
    i32.mul)
  (func (export "run") (param $arg i32) (result i32)
    (local $next i32)
    i32.const 8
    local.get $arg
    i32.store
    local.get $arg
    i32.const 1
    i32.add
    local.tee $next
    call $leaf
    local.get $next
    i32.add))
