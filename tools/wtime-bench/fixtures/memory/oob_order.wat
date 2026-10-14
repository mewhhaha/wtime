(module
  (memory (export "memory") 1 1)
  (data (i32.const 0) "\01\02\03\04")
  (func (export "run") (param i32) (result i32)
    (i32.store8 (i32.const 8) (i32.const 170))
    (memory.copy (i32.const 65534) (i32.const 0) (i32.const 4))
    (i32.store8 (i32.const 9) (i32.const 204))
    (i32.const 0)))
