(module
  (type $padding (array (mut i64)))
  (type $object (struct (field i32)))
  (tag (export "yield") (result (ref $object)))
)
