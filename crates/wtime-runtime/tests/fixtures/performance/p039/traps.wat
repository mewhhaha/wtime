;; SPDX-License-Identifier: Apache-2.0 OR MIT
(module
 (func (export "division_overflow") (result i32) i32.const -2147483648 i32.const -1 i32.div_s)
 (func (export "division_zero") (result i32) i32.const 123 i32.const 0 i32.div_s)
 (func (export "division_success") (result i32) i32.const 84 i32.const 2 i32.div_s)
 (func (export "conversion_nan") (result i32) f64.const nan:0x8000000001234 i32.trunc_f64_s)
 (func (export "conversion_overflow") (result i32) f64.const 2147483648 i32.trunc_f64_s)
 (func (export "conversion_success") (result i32) f64.const 42.75 i32.trunc_f64_s))
