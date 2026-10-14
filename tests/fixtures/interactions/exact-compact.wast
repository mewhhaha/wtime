;; Compact encoding 1, two exact-function imports. Bytes are hand assembled
;; from the pinned compact-import grammar. Readable consumer:
;; (module (type (sub (func (result i32))))
;;   (import "provider" "a" (func (exact 0)))
;;   (import "provider" "b" (func (exact 0)))
;;   (func (export "sum") (type 0) call 0 call 1 i32.add))
(module $good
  (type $parent (sub (func (result i32))))
  (func (export "a") (type $parent) i32.const 20)
  (func (export "b") (type $parent) i32.const 22))
(register "provider")
(module $compact binary
  "\00asm\01\00\00\00"
  "\01\07\01\50\00\60\00\01\7f"
  "\02\15\01\08provider\00\7f\02\01a\20\00\01b\20\00"
  "\03\02\01\00\07\07\01\03sum\00\02"
  "\0a\09\01\07\00\10\00\10\01\6a\0b")
(assert_return (invoke "sum") (i32.const 42))
(module $child
  (type $parent (sub (func (result i32))))
  (type $child (sub $parent (func (result i32))))
  (func (export "a") (type $child) i32.const 20)
  (func (export "b") (type $child) i32.const 22))
(register "provider")
(assert_unlinkable (module binary
  "\00asm\01\00\00\00"
  "\01\07\01\50\00\60\00\01\7f"
  "\02\15\01\08provider\00\7f\02\01a\20\00\01b\20\00"
  "\03\02\01\00\07\07\01\03sum\00\02"
  "\0a\09\01\07\00\10\00\10\01\6a\0b") "incompatible import type")
