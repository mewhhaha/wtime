;; Original P043 custom-page fixture; dual MIT OR Apache-2.0. Source-only, not encoded.
(module
  (memory (export "memory") 256 256 (pagesize 1))
  (data (i32.const 0) "\80\01\02")
  (func (export "read_seed") (result i32 i32)
    (i32.load8_s (i32.const 0))
    (i32.load16_u (i32.const 1)))
  (func (export "write_last_byte") (result i32)
    (i32.store8 (i32.const 255) (i32.const 127))
    (i32.load8_s (i32.const 255)))
  (func (export "last_valid_i16") (result i32)
    (i32.load16_u (i32.const 254)))
  (func (export "oob_i16_at_last_byte") (result i32)
    (i32.load16_u (i32.const 255))))
