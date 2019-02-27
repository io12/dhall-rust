#[macro_export]
macro_rules! include_test_str {
    ($x:expr) => { include_str!(concat!("../dhall-lang/tests/", $x, ".dhall")) };
}

#[macro_export]
macro_rules! include_test_strs_ab {
    ($x:expr) => { (include_test_str!(concat!($x, "A")), include_test_str!(concat!($x, "B"))) };
}

#[macro_export]
macro_rules! run_spec_test {
    (normalization, $path:expr) => {
        let (expr_str, expected_str) = include_test_strs_ab!($path);
        let expr = parser::parse_expr(&expr_str).unwrap();
        let expected = parser::parse_expr(&expected_str).unwrap();
        assert_eq!(normalize::<_, X, _>(&expr), normalize::<_, X, _>(&expected));
    };
}

#[macro_export]
macro_rules! make_spec_test {
    ($type:ident, $name:ident, $path:expr) => {
        #[test]
        #[allow(non_snake_case)]
        fn $name(){
            use dhall::*;
            run_spec_test!($type, $path);
        }
    };
}
