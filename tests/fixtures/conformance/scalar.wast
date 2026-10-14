;; Locally authored, CC0; expected value follows integer addition.
(module $a
  (global (export "g") (mut i32) (i32.const 41))
  (func (export "answer") (result i32) global.get 0 i32.const 1 i32.add))
(register "a" $a)
(assert_return (invoke $a "answer") (i32.const 42))
(module (import "a" "g" (global (mut i32)))
  (func (export "g") (result i32) global.get 0))
(assert_return (invoke "g") (i32.const 41))
(assert_trap (module (func unreachable) (start 0)) "unreachable")
(assert_invalid (module (func (result i32))) "type mismatch")
(assert_malformed (module binary "\00") "unexpected end")
