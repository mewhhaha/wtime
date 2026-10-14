(module
  (func (export "run") (result i32)
    (local $counter i32)
    (local $sum i32)
    (local $step i32)
    i32.const 0
    local.set $counter
    i32.const 0
    local.set $sum
    i32.const 8
    local.set $step
    block $exit
      loop $head
        local.get $counter
        i32.const 4
        i32.ge_s
        br_if $exit
        local.get $sum
        local.get $step
        i32.add
        local.set $sum
        local.get $counter
        i32.const 1
        i32.add
        local.set $counter
        br $head
      end
    end
    local.get $sum))
