;; CC0-1.0. Original P036 validation/factory fixture; unencoded staging draft.
(module
  (type $s (struct (field i32)))
  (func (export "run") (result i32)
    (block $out (result (ref $s))
      (br_on_cast $out (ref any) (ref $s)
        (struct.new $s (i32.const 7)))
      unreachable)
    struct.get $s 0))
