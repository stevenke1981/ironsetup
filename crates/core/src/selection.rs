//! Bounded boolean selection parser: NOT > AND > OR/whitespace lists.
use anyhow::{bail, ensure, Result};
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Name(String),
    And,
    Or,
    Not,
    Open,
    Close,
}
pub fn evaluate_condition(expression: &str, selected: &BTreeSet<String>) -> Result<bool> {
    if expression.trim().is_empty() {
        return Ok(true);
    }
    ensure!(
        expression.len() <= 8192,
        "Selection expression exceeds 8192 bytes"
    );
    let spaced = expression.replace('(', " ( ").replace(')', " ) ");
    let mut tokens = Vec::new();
    for word in spaced.split_whitespace() {
        let lower = word.to_lowercase();
        tokens.push(match lower.as_str() {
            "and" => Token::And,
            "or" => Token::Or,
            "not" => Token::Not,
            "(" => Token::Open,
            ")" => Token::Close,
            _ => {
                ensure!(
                    word.chars()
                        .all(|c| c.is_alphanumeric() || "_-.\\/".contains(c)),
                    "Invalid selection name: {word}"
                );
                Token::Name(lower)
            }
        });
    }
    let normalized = selected
        .iter()
        .map(|x| x.to_lowercase())
        .collect::<BTreeSet<_>>();
    let mut p = Parser {
        tokens: &tokens,
        at: 0,
        selected: &normalized,
    };
    let result = p.or(0)?;
    ensure!(
        p.at == tokens.len(),
        "Unexpected token in selection expression"
    );
    Ok(result)
}
struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    selected: &'a BTreeSet<String>,
}
impl Parser<'_> {
    fn or(&mut self, depth: usize) -> Result<bool> {
        let mut value = self.and(depth)?;
        loop {
            match self.tokens.get(self.at) {
                Some(Token::Or) => self.at += 1,
                Some(Token::Name(_) | Token::Open | Token::Not) => (),
                _ => break,
            }
            let rhs = self.and(depth)?;
            value |= rhs;
        }
        Ok(value)
    }
    fn and(&mut self, depth: usize) -> Result<bool> {
        let mut value = self.atom(depth)?;
        while self.tokens.get(self.at) == Some(&Token::And) {
            self.at += 1;
            let rhs = self.atom(depth)?;
            value &= rhs;
        }
        Ok(value)
    }
    fn atom(&mut self, depth: usize) -> Result<bool> {
        ensure!(depth < 64, "Selection expression nesting is too deep");
        let Some(token) = self.tokens.get(self.at) else {
            bail!("Incomplete selection expression");
        };
        self.at += 1;
        match token {
            Token::Name(name) => Ok(self.selected.contains(name)),
            Token::Not => Ok(!self.atom(depth + 1)?),
            Token::Open => {
                let v = self.or(depth + 1)?;
                ensure!(
                    self.tokens.get(self.at) == Some(&Token::Close),
                    "Unclosed selection parenthesis"
                );
                self.at += 1;
                Ok(v)
            }
            _ => bail!("Expected a component/task name"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn yes(s: &str) -> bool {
        evaluate_condition(s, &["core".into(), "Docs".into()].into_iter().collect()).unwrap()
    }
    #[test]
    fn precedence() {
        assert!(yes("core and not extra"));
        assert!(!yes("core and extra"));
        assert!(yes("extra or core and docs"));
        assert!(!yes("not (core or extra)"));
    }
    #[test]
    fn names_lists() {
        assert!(yes("DOCS"));
        assert!(yes("missing core"));
        assert!(yes(""));
    }
    #[test]
    fn syntax_is_not_short_circuited() {
        for e in [
            "core or",
            "missing and",
            "core)",
            "(core",
            "a && b",
            "and core",
        ] {
            assert!(evaluate_condition(e, &BTreeSet::new()).is_err(), "{e}");
        }
    }
}
