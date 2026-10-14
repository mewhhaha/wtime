;; SPDX-License-Identifier: CC0-1.0
;; The root is supplied as a ref parameter; run performs no allocation or ref
;; operation before the zero-argument host collection barrier.
(module
  (type $box (struct (field i32)))
  (import "env" "collect" (func $collect))
  (func (export "make") (param $x i32) (result (ref $box))
    local.get $x
    struct.new $box)
  (func (export "run") (param $root (ref $box)) (result i32)
    (local $keep (ref $box))
    local.get $root       ;; pc 0
    local.set $keep       ;; pc 1
    i32.const 4           ;; pc 2
    i32.const 5           ;; pc 3
    i32.add               ;; pc 4
    drop                  ;; pc 5
    call $collect         ;; pc 6: first barrier; root only in active local(s)
    local.get $keep       ;; pc 7: decoded suffix
    struct.get $box 0))
