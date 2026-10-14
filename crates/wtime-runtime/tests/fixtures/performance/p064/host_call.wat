;; P064 host callback with exported guest child for host reentry. MIT OR Apache-2.0.
(module
  (import "env" "callback" (func $callback (param i32) (result i32)))
  (memory (export "memory") 1 1)
  (func (export "child") (param $x i32) (result i32)
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
    call $callback
    local.get $next
    i32.add))
