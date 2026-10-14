;; SPDX-License-Identifier: CC0-1.0
;; Root-authored unencoded direct-reference-parameter fallback fixture.
(module
  (type $s (struct (field i32)))
  (type $u (struct (field i64)))
  (func $identity (export "identity") (param (ref $s)) (result i32)
    local.get 0
    struct.get $s 0)
  (func (export "run") (result i32)
    i32.const 41
    struct.new $s
    call $identity)
  (func (export "make_s") (result (ref $s))
    i32.const 41
    struct.new $s)
  (func (export "make_u") (result (ref $u))
    i64.const 99
    struct.new $u))
