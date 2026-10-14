(module
  (memory (export "memory") 1 1)
  (func (export "run") (param i32) (result i32)
    (i32.store8 (i32.const 8) (i32.const 170))
    (drop (i32.load offset=4294967295 (i32.const 1)))
    (i32.store8 (i32.const 9) (i32.const 204))
    (i32.const 0)))
