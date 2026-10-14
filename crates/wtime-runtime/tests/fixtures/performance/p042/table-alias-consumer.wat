;; Original P042 table alias consumer fixture; dual MIT OR Apache-2.0.
;; Instantiate in the same Store against table-alias-provider.shared.
;; Source-only: parser validation, encoding and execution are pending.
(module
  (type $zero_i32 (func (result i32)))
  (import "provider" "shared" (table $shared 2 4 funcref))
  (func $c30 (type $zero_i32) (result i32) i32.const 30)
  (elem declare func $c30)
  (func (export "run") (param $index i32) (result i32)
    (call_indirect $shared (type $zero_i32) (local.get $index)))
  (func (export "set_c30") (param $index i32)
    local.get $index
    ref.func $c30
    table.set $shared)
  (func (export "grow_c30") (param $delta i32) (result i32)
    ref.func $c30
    local.get $delta
    table.grow $shared)
  (func (export "size") (result i32)
    table.size $shared))
