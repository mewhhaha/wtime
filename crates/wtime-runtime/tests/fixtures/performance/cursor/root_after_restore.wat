(module
  (type $box (struct (field (mut i32))))
  (import "host" "collect" (func $collect))
  (func (export "run") (result i32)
    (local $root (ref null $box))
    ;; Source instruction PCs: 0 i32.const 42; 1 struct.new $box;
    ;; 2 local.set $root; 3 call $collect; 4 local.get $root;
    ;; 5 struct.get $box 0; 6 function end.
    i32.const 42
    struct.new $box
    local.set $root
    call $collect
    local.get $root
    struct.get $box 0))
