core.wasm is generated from core.wat using wasm-tools parse (1.261.0).
Assertions follow core integer modular arithmetic, IEEE nearest-even and
signed-zero min rules, checked memory access, and specified growth sentinel.
Regenerate: wasm-tools parse core.wat -o core.wasm
