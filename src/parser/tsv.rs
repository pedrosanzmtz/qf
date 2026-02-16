use serde_json::Value;

use crate::error::QfError;

pub fn parse(input: &str) -> Result<Value, QfError> {
    super::csv::parse_delimited(input, b'\t')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_tsv() {
        let input = "name\tage\tcity\nAlice\t30\tNYC\nBob\t25\tLA\n";
        let val = parse(input).unwrap();
        let arr = val.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "Alice");
        assert_eq!(arr[1]["age"], "25");
    }

    #[test]
    fn fields_with_spaces() {
        let input = "name\tdescription\nAlice\thas spaces here\n";
        let val = parse(input).unwrap();
        assert_eq!(val[0]["description"], "has spaces here");
    }
}
