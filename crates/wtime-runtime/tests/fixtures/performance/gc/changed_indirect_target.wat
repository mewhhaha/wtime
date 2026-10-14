(module
  (type $base (sub (func (result i32))))
  (type $child (sub $base (func (result i32))))
  (type $bad (func (param i32) (result i32)))

  (func $first (type $child) (result i32)
    i32.const 11)
  (func $second (type $base) (result i32)
    i32.const 22)
  (func $bad_target (type $bad) (param i32) (result i32)
    local.get 0)

  (table $table (export "table") 3 funcref)
  (elem (i32.const 0) func $first $second $bad_target)

  (func (export "run") (result i32)
    i32.const 0
    call_indirect (type $base))
  (func (export "set_first")
    i32.const 0
    ref.func $first
    table.set $table)
  (func (export "set_second")
    i32.const 0
    ref.func $second
    table.set $table)
  (func (export "set_bad")
    i32.const 0
    ref.func $bad_target
    table.set $table))
