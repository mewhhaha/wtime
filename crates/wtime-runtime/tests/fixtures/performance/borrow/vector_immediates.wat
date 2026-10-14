(module
  (func (export "table") (param i32) (result i32)
    (block $out (result i32)
      i32.const 42
      local.get 0
      br_table $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out $out))
  (func (export "select") (param i32) (result i32)
    i32.const 7 i32.const 9 local.get 0 select (result i32)))
