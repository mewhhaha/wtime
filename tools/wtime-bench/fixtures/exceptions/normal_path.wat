(module
  (tag $number (param i32))
  (func (export "run") (param $value i32) (result i32)
    (block $caught (result i32)
      (try_table (result i32) (catch $number $caught)
        (local.get $value)))))
