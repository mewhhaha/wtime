;; Original P041 regression source; dual MIT OR Apache-2.0.
;; No encoding, validation, or guest execution has been performed yet.
(module
  (type $box (struct (field i32)))
  (import "env" "host" (func $host (param i32) (result i32)))
  (func $numeric (param i32) (result i32)
    local.get 0
    call $host)
  (func $run (export "run") (result i32)
    (local $root (ref null $box))
    i32.const 91
    struct.new $box
    local.set $root
    i32.const 5
    call $numeric
    drop
    local.get $root
    struct.get $box 0))
