const POLYFILLS: &str = include_str!("polyfills.js");

pub fn script() -> &'static str {
    POLYFILLS
}
