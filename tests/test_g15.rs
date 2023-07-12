use std::fs;
#[path = "../src/io/g15.rs"]
pub mod g15;

pub fn read_log(path: &str) -> String {
    let g15_log =
        fs::read_to_string(format!("tests/data/{}.log", path)).expect("No g15 log file found?");
    g15_log
}

pub fn read_expected(path: &str) -> String {
    let g15_expected = fs::read_to_string(format!("tests/data/{}_expected.log", path))
        .expect("No g15 log file found?");
    g15_expected
}

#[cfg(test)]
mod tests {
    use crate::g15::G15Parser;

    use super::*;

    #[test]
    fn test_normal() {
        let path = "normal";
        let log = read_log(path);
        let parser = G15Parser::new();
        let players = parser.parse_g15(&log);
        // println!("{:?}", players);
        let expected = read_expected(path);
        assert!(expected == format!("{:?}", players));
    }

    #[test]
    fn test_bad_int() {
        let path = "bad_int";
        let log = read_log(path);
        let parser = G15Parser::new();
        let players = parser.parse_g15(&log);
        // println!("{:?}", players);
        let expected = read_expected(path);
        assert!(expected == format!("{:?}", players));
    }

    #[test]
    fn test_bad_idx() {
        let path = "bad_idx";
        let log = read_log(path);
        let parser = G15Parser::new();
        let players = parser.parse_g15(&log);
        // println!("{:?}", players);
        let expected = read_expected(path);
        assert!(expected == format!("{:?}", players));
    }

    #[test]
    fn test_none() {
        let path = "none";
        let log = read_log(path);
        let parser = G15Parser::new();
        let players = parser.parse_g15(&log);
        // println!("{:?}", players);
        let expected = read_expected(path);
        assert!(expected == format!("{:?}", players));
    }
}
