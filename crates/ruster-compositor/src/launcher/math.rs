//! Arithmetic in the launcher.
//!
//! The second provider, and the reason it ships with the first: a trait with one
//! implementation has never been tested. This one is deliberately unlike the app
//! provider — its answers are *computed* rather than matched, there is exactly
//! one of them, and it is either certain or silent — which is what makes it
//! evidence that [`Provider`](super::Provider) is an abstraction rather than an
//! app launcher wearing one.
//!
//! Recursive descent, no dependency. The parser is pure and takes a string, so
//! all of it is testable without a compositor.

use super::{Activation, Candidate, Provider, ProviderCtx};
use ruster_picker::CONFIDENCE_MAX;

/// Evaluate an arithmetic expression, or `None` when the text is not one.
///
/// `None` is the common case, not the error case: every keystroke in the
/// launcher is offered to this provider, and almost none of them are sums.
pub fn eval(expr: &str) -> Option<f64> {
    let mut p = Parser {
        chars: expr.chars().filter(|c| !c.is_whitespace()).collect(),
        at: 0,
    };
    let value = p.expr()?;
    if p.at != p.chars.len() {
        return None; // trailing junk: "2+2 apples" is not a sum
    }
    // The single rejection of a non-answer, covering division and modulo by
    // zero, `ln(0)`, and overflow alike. Guarding each operator as well was
    // redundant — mutation testing removed those checks and no test noticed,
    // which is what redundant means.
    value.is_finite().then_some(value)
}

/// Format a result the way a person writes it: no trailing `.0`, no float noise.
pub fn format_result(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let s = format!("{value:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.at += 1;
            return true;
        }
        false
    }

    fn expr(&mut self) -> Option<f64> {
        let mut lhs = self.term()?;
        loop {
            if self.eat('+') {
                lhs += self.term()?;
            } else if self.eat('-') {
                lhs -= self.term()?;
            } else {
                return Some(lhs);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut lhs = self.unary()?;
        loop {
            if self.eat('*') {
                lhs *= self.unary()?;
            } else if self.eat('/') {
                lhs /= self.unary()?;
            } else if self.eat('%') {
                lhs %= self.unary()?;
            } else {
                return Some(lhs);
            }
        }
    }

    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            return Some(-self.unary()?);
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.eat('^') {
            // Right-associative: `2^3^2` is 2^(3^2) = 512, not (2^3)^2 = 64.
            // Recursing through `unary` rather than `power` is also what lets
            // `2^-1` parse.
            let exp = self.unary()?;
            return Some(base.powf(exp));
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        if self.eat('(') {
            let inner = self.expr()?;
            return self.eat(')').then_some(inner);
        }
        if matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
            let start = self.at;
            while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
                self.at += 1;
            }
            let name: String = self.chars[start..self.at].iter().collect();
            if self.eat('(') {
                let arg = self.expr()?;
                if !self.eat(')') {
                    return None;
                }
                return apply(&name, arg);
            }
            return match name.as_str() {
                "pi" => Some(std::f64::consts::PI),
                "e" => Some(std::f64::consts::E),
                _ => None,
            };
        }
        let start = self.at;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '.') {
            self.at += 1;
        }
        if start == self.at {
            return None;
        }
        self.chars[start..self.at]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }
}

fn apply(name: &str, arg: f64) -> Option<f64> {
    let out = match name {
        "sqrt" => arg.sqrt(),
        "abs" => arg.abs(),
        "floor" => arg.floor(),
        "ceil" => arg.ceil(),
        "round" => arg.round(),
        "ln" => arg.ln(),
        "log" => arg.log10(),
        "log2" => arg.log2(),
        "exp" => arg.exp(),
        "sin" => arg.sin(),
        "cos" => arg.cos(),
        "tan" => arg.tan(),
        _ => return None,
    };
    out.is_finite().then_some(out)
}

/// Whether the text is worth handing to the parser at all.
///
/// Without this, typing `5` offers a row reading "5 = 5", and every bare number
/// a user types on the way to something else produces a result they did not ask
/// for. An expression needs an operator or a bracket to be an expression.
fn looks_like_a_sum(query: &str) -> bool {
    query.chars().any(|c| "+-*/%^(".contains(c))
        && query
            .chars()
            .any(|c| c.is_ascii_digit() || c == 'p' || c == 'e')
}

#[derive(Default)]
pub struct MathProvider;

impl Provider for MathProvider {
    fn name(&self) -> &str {
        "maths"
    }

    fn query(&mut self, query: &str, _ctx: &ProviderCtx, _limit: usize) -> Vec<Candidate> {
        if !looks_like_a_sum(query) {
            return Vec::new();
        }
        let Some(value) = eval(query) else {
            return Vec::new();
        };
        let answer = format_result(value);
        vec![Candidate {
            label: answer.clone(),
            detail: format!("= {query}"),
            // Certain. A sum is right or it is absent, so it outranks any fuzzy
            // app match — which is what a user typing `6*7` expects to see first.
            score: CONFIDENCE_MAX,
            activation: Activation::Copy(answer),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_is_not_left_to_right() {
        assert_eq!(eval("2+3*4"), Some(14.0), "not 20");
        assert_eq!(eval("(2+3)*4"), Some(20.0));
        assert_eq!(eval("10-2-3"), Some(5.0), "subtraction stays left-assoc");
    }

    #[test]
    fn exponentiation_is_right_associative() {
        // 2^(3^2) = 512, not (2^3)^2 = 64. Getting this wrong gives an answer
        // that looks entirely plausible, which is why it is asserted.
        assert_eq!(eval("2^3^2"), Some(512.0));
        assert_eq!(eval("2^-1"), Some(0.5), "and a negative exponent parses");
    }

    #[test]
    fn dividing_by_nothing_is_not_an_answer() {
        assert_eq!(eval("1/0"), None, "inf is not a result");
        assert_eq!(eval("5%0"), None);
    }

    #[test]
    fn text_that_is_not_a_sum_is_refused() {
        assert_eq!(eval("firefox"), None);
        assert_eq!(eval("2+2 apples"), None, "trailing junk is not a sum");
        assert_eq!(eval(""), None);
        assert_eq!(eval("()"), None);
    }

    #[test]
    fn functions_and_constants_evaluate() {
        assert_eq!(eval("sqrt(16)"), Some(4.0));
        assert_eq!(eval("floor(3.7)"), Some(3.0));
        assert_eq!(eval("ln(0)"), None, "-inf is not a result");
        assert!((eval("pi").unwrap() - std::f64::consts::PI).abs() < 1e-12);
    }

    #[test]
    fn a_bare_number_offers_nothing() {
        // Typing `5` on the way to `5+5` must not produce a "5 = 5" row.
        let mut p = MathProvider;
        assert!(p.query("5", &ProviderCtx::default(), 10).is_empty());
        assert!(p.query("firefox", &ProviderCtx::default(), 10).is_empty());

        let hit = p.query("5+5", &ProviderCtx::default(), 10);
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].label, "10");
        assert_eq!(hit[0].score, CONFIDENCE_MAX);
        assert_eq!(hit[0].activation, Activation::Copy("10".into()));
    }

    #[test]
    fn results_read_the_way_a_person_writes_them() {
        assert_eq!(format_result(42.0), "42");
        assert_eq!(format_result(0.5), "0.5");
        assert_eq!(format_result(-3.0), "-3");
        assert_eq!(eval("10/4").map(format_result).unwrap(), "2.5");
    }
}
