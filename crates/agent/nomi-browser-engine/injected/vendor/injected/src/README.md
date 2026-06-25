# Vendored Injected Sources

This directory contains helper JavaScript/TypeScript sources that are injected
into browser pages by `nomi-browser-engine`.

The sources are vendored so the Rust crate can generate compile-time constants
without a network step during normal builds. After updating the vendored files,
run the generator documented in `utils/generate_injected` and review the
generated Rust output before committing.
