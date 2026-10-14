;; CC0-1.0. Source: staged specifically for P029, 2026-10-11.
(module
  (type $unary (func (param i32) (result i32)))
  (func $descend (type $unary) (param i32) (result i32)
    local.get 0
    i32.eqz
    if (result i32)
      i32.const 23
    else
      local.get 0
      i32.const 1
      i32.sub
      return_call $descend
    end)
  (func (export "run") (param i32) (result i32)
    local.get 0
    call $descend))
