use trycmd::TestCases;

#[test]
fn cli_tests() {
    TestCases::new().case("tests/test.toml");
}
