;; CC0-1.0. Source: staged specifically for P029, 2026-10-11.
(module
  (type $node (struct (field i32)))
  (type $visit-type (func (param (ref $node))))
  (import "host" "visit" (func $visit (type $visit-type)))
  (func (export "read") (param (ref $node)) (result i32)
    local.get 0
    struct.get $node 0)
  (func (export "run") (result i32)
    (local $root (ref $node))
    i32.const 41
    struct.new $node
    local.set $root
    local.get $root
    call $visit
    local.get $root
    call 1))
