;; An unshared memory with one-byte pages: a write, a one-byte growth, then an
;; atomic access, which converts the memory to a shared backing.
(module
  (memory (export "m") 4 100 (pagesize 1))
  (func (export "grow") (param i32) (result i32) local.get 0 memory.grow)
  (func (export "load") (param i32) (result i32) local.get 0 i32.load)
  (func (export "aload") (param i32) (result i32) local.get 0 i32.atomic.load))
