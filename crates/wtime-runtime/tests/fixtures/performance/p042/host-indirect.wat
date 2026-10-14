;; Original P042 host call_ref/indirect/tail source fixture.
;; Dual MIT OR Apache-2.0. Source-only; host callback contract is in manual-oracles.json.
(module
  (type $unary (func (param i32) (result i32)))
  (type $box (struct (field i32)))
  (import "env" "host" (func $host (type $unary) (param i32) (result i32)))
  (table $t 1 funcref)
  (elem (table $t) (i32.const 0) func $host)
  (elem declare func $host)
  (func $indirect (export "indirect") (param $arg i32) (result i32)
    (call_indirect $t (type $unary) (local.get $arg) (i32.const 0)))
  (func (export "tail_indirect") (param $arg i32) (result i32)
    (return_call_indirect $t (type $unary) (local.get $arg) (i32.const 0)))
  (func (export "by_ref") (param $arg i32) (result i32)
    (call_ref $unary (local.get $arg) (ref.func $host)))
  (func (export "tail_by_ref") (param $arg i32) (result i32)
    (return_call_ref $unary (local.get $arg) (ref.func $host)))
  (func (export "root") (result i32)
    (local $root (ref null $box))
    i32.const 91
    struct.new $box
    local.set $root
    i32.const 5
    call $indirect
    drop
    local.get $root
    struct.get $box 0))
