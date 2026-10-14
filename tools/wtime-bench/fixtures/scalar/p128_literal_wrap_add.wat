(module
  (func (export "run") (param $n i32) (result i32)
    (local $remaining i32)
    (local $acc i32)
    local.get $n
    local.set $remaining
    block $done
      loop $again
        local.get $remaining
        i32.eqz
        br_if $done
        i32.const 2147483647
        i32.const 1
        i32.add
        local.get $acc
        i32.add
        local.set $acc
        local.get $remaining
        i32.const 1
        i32.sub
        local.set $remaining
        br $again
      end
    end
    local.get $acc))
