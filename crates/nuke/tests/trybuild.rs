//! trybuild compile-fail tests for the type-level invariants nuke
//! relies on. Each case is a `tests/compile_fail/*.rs` file that
//! intentionally fails to compile; trybuild verifies the failure
//! happens (without asserting on the exact error message - those
//! drift across rustc versions).

#[test]
#[ignore = "trybuild compile-fail cases - run with `cargo test --test trybuild -- --ignored`. \
            Skipped by default to keep the regular test suite stable across rustc versions."]
fn type_level_invariants_must_fail_to_compile() {
    let runner = trybuild::TestCases::new();
    runner.compile_fail("tests/compile_fail/*.rs");
}
