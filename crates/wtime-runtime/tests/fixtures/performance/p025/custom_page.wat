(module
  ;; 256 one-byte pages make a 256-byte memory with a non-default page size.
  (memory (export "memory") 256 256 (pagesize 1))
  (func (export "run") (param $address i32) (result i32)
    (i32.store8 (local.get $address) (i32.const -128))
    (i32.load8_s (local.get $address))))
