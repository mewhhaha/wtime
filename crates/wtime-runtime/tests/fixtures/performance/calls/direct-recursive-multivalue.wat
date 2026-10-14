;; CC0-1.0. Source: staged specifically for P029, 2026-10-11.
(module
  (type $pair (func (param i32 i64) (result i32 i64)))
  (func $descend (type $pair) (param i32 i64) (result i32 i64)
    local.get 0
    i32.eqz
    if (result i32 i64)
      i32.const 7
      i64.const 9
    else
      local.get 0
      i32.const 1
      i32.sub
      local.get 1
      call $descend
    end)
  (func (export "run") (result i32 i64)
    i32.const 64
    i64.const 5
    call $descend))
