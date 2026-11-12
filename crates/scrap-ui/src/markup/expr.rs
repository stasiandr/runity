//! What goes between `{{` and `}}`: a value from the game's model, and a
//! little arithmetic on it — `player.hp / player.max * 100`,
//! `orders.length > 0`, `!round.over`. Nothing more: no calls, no
//! assignment, no `if`. The logic is the game's Rust (DNA, postulate 5);
//! a binding only says how a value the game gave is shown. A screen that
//! wants more asks the game for one more value in its model.

use serde_json::{Number, Value};

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value),
    /// `player.hp`: names from the model, or a loop's own name first.
    Path(Vec<String>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

impl Op {
    fn binding(self) -> u8 {
        match self {
            Op::Or => 1,
            Op::And => 2,
            Op::Eq | Op::Ne => 3,
            Op::Lt | Op::Le | Op::Gt | Op::Ge => 4,
            Op::Add | Op::Sub => 5,
            Op::Mul | Op::Div | Op::Rem => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Str(String),
    Name(String),
    Op(Op),
    Not,
    Open,
    Close,
    Dot,
}

fn tokens(src: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            c if c.is_whitespace() => i += 1,
            '0'..='9' => {
                let start = i;
                while i < chars.len() {
                    // A dot is a decimal point only with a digit after it:
                    // `orders.0.name` is a path.
                    let decimal =
                        chars[i] == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit());
                    if !(chars[i].is_ascii_digit() || decimal) {
                        break;
                    }
                    i += 1;
                }
                let n: String = chars[start..i].iter().collect();
                out.push(Token::Number(
                    n.parse().map_err(|_| format!("`{n}` is not a number"))?,
                ));
            }
            '\'' | '"' => {
                let end = chars[i + 1..]
                    .iter()
                    .position(|d| *d == c)
                    .ok_or_else(|| "a string that is never closed".to_string())?;
                out.push(Token::Str(chars[i + 1..i + 1 + end].iter().collect()));
                i += end + 2;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Token::Name(chars[start..i].iter().collect()));
            }
            '.' => {
                out.push(Token::Dot);
                i += 1;
            }
            '(' => {
                out.push(Token::Open);
                i += 1;
            }
            ')' => {
                out.push(Token::Close);
                i += 1;
            }
            _ => {
                let (token, width) = match (c, next) {
                    ('*', _) => (Token::Op(Op::Mul), 1),
                    ('/', _) => (Token::Op(Op::Div), 1),
                    ('%', _) => (Token::Op(Op::Rem), 1),
                    ('+', _) => (Token::Op(Op::Add), 1),
                    ('-', _) => (Token::Op(Op::Sub), 1),
                    ('<', Some('=')) => (Token::Op(Op::Le), 2),
                    ('<', _) => (Token::Op(Op::Lt), 1),
                    ('>', Some('=')) => (Token::Op(Op::Ge), 2),
                    ('>', _) => (Token::Op(Op::Gt), 1),
                    ('=', Some('=')) => (Token::Op(Op::Eq), 2),
                    ('!', Some('=')) => (Token::Op(Op::Ne), 2),
                    ('!', _) => (Token::Not, 1),
                    ('&', Some('&')) => (Token::Op(Op::And), 2),
                    ('|', Some('|')) => (Token::Op(Op::Or), 2),
                    ('=', _) => {
                        return Err("`=` assigns, and a binding only reads: `==` compares".into())
                    }
                    ('?', _) => {
                        return Err(
                            "no `? :` in a binding: let the game put the value in the model".into(),
                        )
                    }
                    _ => return Err(format!("`{c}` cannot be read in a binding")),
                };
                if (c == '=' || c == '!') && chars.get(i + 2) == Some(&'=') {
                    // `===` and `!==` read as `==` and `!=`.
                    i += 1;
                }
                out.push(token);
                i += width;
            }
        }
    }
    Ok(out)
}

/// Read one expression.
pub fn parse(src: &str) -> Result<Expr, String> {
    let tokens = tokens(src)?;
    if tokens.is_empty() {
        return Err("an empty `{{ }}`".into());
    }
    let mut p = Parser { tokens, i: 0 };
    let e = p.expr(0)?;
    if p.i != p.tokens.len() {
        return Err(format!(
            "`{src}`: something after the end of the expression"
        ));
    }
    Ok(e)
}

struct Parser {
    tokens: Vec<Token>,
    i: usize,
}

impl Parser {
    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.i).cloned();
        self.i += 1;
        t
    }

    fn expr(&mut self, min: u8) -> Result<Expr, String> {
        let mut left = self.unary()?;
        while let Some(Token::Op(op)) = self.tokens.get(self.i).cloned() {
            if op.binding() <= min {
                break;
            }
            self.i += 1;
            let right = self.expr(op.binding())?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Token::Not) => Ok(Expr::Not(Box::new(self.unary()?))),
            Some(Token::Op(Op::Sub)) => Ok(Expr::Neg(Box::new(self.unary()?))),
            Some(Token::Number(n)) => Ok(Expr::Literal(number(n))),
            Some(Token::Str(s)) => Ok(Expr::Literal(Value::String(s))),
            Some(Token::Open) => {
                let e = self.expr(0)?;
                match self.next() {
                    Some(Token::Close) => Ok(e),
                    _ => Err("a `(` that is never closed".into()),
                }
            }
            Some(Token::Name(name)) => {
                match name.as_str() {
                    "true" => return Ok(Expr::Literal(Value::Bool(true))),
                    "false" => return Ok(Expr::Literal(Value::Bool(false))),
                    "null" => return Ok(Expr::Literal(Value::Null)),
                    _ => {}
                }
                let mut path = vec![name];
                while self.tokens.get(self.i) == Some(&Token::Dot) {
                    self.i += 1;
                    match self.next() {
                        Some(Token::Name(n)) => path.push(n),
                        Some(Token::Number(n)) if n.fract() == 0.0 => path.push(n.to_string()),
                        _ => return Err("a name after the `.`".into()),
                    }
                }
                if self.tokens.get(self.i) == Some(&Token::Open) {
                    return Err(format!(
                        "`{}(`: a binding calls nothing — the logic is the game's",
                        path.join(".")
                    ));
                }
                Ok(Expr::Path(path))
            }
            Some(other) => Err(format!("{other:?} where a value goes")),
            None => Err("the expression ends where a value goes".into()),
        }
    }
}

fn number(n: f64) -> Value {
    Number::from_f64(n).map_or(Value::Null, Value::Number)
}

/// Whether a value counts as yes: as in JavaScript, `0`, `""`, `false`,
/// nothing and an empty list do not.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(_) => true,
    }
}

/// A value as text on the screen: a whole number without a point, others
/// to two places at most.
pub fn show(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            let n = n.as_f64().unwrap_or(0.0);
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", n as i64)
            } else {
                let s = format!("{n:.2}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => v.to_string(),
    }
}

fn as_number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::Bool(b) => *b as u8 as f64,
        Value::String(s) => s.trim().parse().unwrap_or(f64::NAN),
        Value::Null => 0.0,
        _ => f64::NAN,
    }
}

/// Where names are looked up: the loops a binding is inside, innermost
/// last, then the model.
pub struct Scope<'a> {
    pub model: &'a Value,
    pub locals: &'a [(String, Value)],
}

impl Scope<'_> {
    /// The value at a path, and whether it was there at all — a screen
    /// says which of its paths the game never filled.
    pub fn get(&self, path: &[String]) -> Option<Value> {
        let (first, rest) = path.split_first()?;
        let mut v = match self.locals.iter().rev().find(|(n, _)| n == first) {
            Some((_, v)) => v.clone(),
            None => self.model.get(first)?.clone(),
        };
        for name in rest {
            v = match (&v, name.as_str()) {
                (Value::Array(a), "length") => Value::from(a.len()),
                (Value::String(s), "length") => Value::from(s.chars().count()),
                (Value::Array(a), i) => a.get(i.parse::<usize>().ok()?)?.clone(),
                (Value::Object(o), n) => o.get(n)?.clone(),
                _ => return None,
            };
        }
        Some(v)
    }
}

/// Work an expression out. A path the model does not have is nothing, and
/// is added to `missing`.
pub fn eval(e: &Expr, scope: &Scope, missing: &mut Vec<String>) -> Value {
    match e {
        Expr::Literal(v) => v.clone(),
        Expr::Path(path) => scope.get(path).unwrap_or_else(|| {
            missing.push(path.join("."));
            Value::Null
        }),
        Expr::Not(e) => Value::Bool(!truthy(&eval(e, scope, missing))),
        Expr::Neg(e) => number(-as_number(&eval(e, scope, missing))),
        Expr::Binary(op, a, b) => {
            let a = eval(a, scope, missing);
            match op {
                Op::And => {
                    return if truthy(&a) {
                        eval(b, scope, missing)
                    } else {
                        a
                    }
                }
                Op::Or => {
                    return if truthy(&a) {
                        a
                    } else {
                        eval(b, scope, missing)
                    }
                }
                _ => {}
            }
            let b = eval(b, scope, missing);
            let (x, y) = (as_number(&a), as_number(&b));
            match op {
                Op::Add => match (&a, &b) {
                    (Value::String(_), _) | (_, Value::String(_)) => {
                        Value::String(show(&a) + &show(&b))
                    }
                    _ => number(x + y),
                },
                Op::Sub => number(x - y),
                Op::Mul => number(x * y),
                Op::Div => number(x / y),
                Op::Rem => number(x % y),
                Op::Lt => Value::Bool(x < y),
                Op::Le => Value::Bool(x <= y),
                Op::Gt => Value::Bool(x > y),
                Op::Ge => Value::Bool(x >= y),
                Op::Eq => Value::Bool(same(&a, &b)),
                Op::Ne => Value::Bool(!same(&a, &b)),
                Op::And | Op::Or => unreachable!("answered above"),
            }
        }
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(_), _) | (_, Value::Number(_)) => as_number(a) == as_number(b),
        _ => a == b,
    }
}

/// Text with `{{ }}` in it: `"{{hp}} / {{max}}"`, `"{{hp / max * 100}}%"`.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub pieces: Vec<Piece>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Text(String),
    Expr(Expr),
}

impl Template {
    pub fn parse(src: &str) -> Result<Self, String> {
        let mut pieces = Vec::new();
        let mut rest = src;
        while let Some(open) = rest.find("{{") {
            if open > 0 {
                pieces.push(Piece::Text(rest[..open].to_string()));
            }
            let close = rest[open..]
                .find("}}")
                .ok_or_else(|| format!("`{src}`: a `{{{{` that is never closed"))?;
            let inner = &rest[open + 2..open + close];
            pieces.push(Piece::Expr(
                parse(inner).map_err(|e| format!("`{{{{{inner}}}}}`: {e}"))?,
            ));
            rest = &rest[open + close + 2..];
        }
        if !rest.is_empty() {
            pieces.push(Piece::Text(rest.to_string()));
        }
        Ok(Self { pieces })
    }

    /// Whether it has any `{{ }}`: one without is plain text.
    pub fn is_bound(&self) -> bool {
        self.pieces.iter().any(|p| matches!(p, Piece::Expr(_)))
    }

    /// The value of a template that is one `{{ }}` and nothing else, as
    /// it is — a number stays a number, a list a list; text otherwise.
    pub fn value(&self, scope: &Scope, missing: &mut Vec<String>) -> Value {
        match self.pieces.as_slice() {
            [Piece::Expr(e)] => eval(e, scope, missing),
            _ => Value::String(self.text(scope, missing)),
        }
    }

    pub fn text(&self, scope: &Scope, missing: &mut Vec<String>) -> String {
        let mut out = String::new();
        for p in &self.pieces {
            match p {
                Piece::Text(t) => out.push_str(t),
                Piece::Expr(e) => out.push_str(&show(&eval(e, scope, missing))),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(src: &str, model: &Value) -> Value {
        let mut missing = Vec::new();
        let scope = Scope { model, locals: &[] };
        eval(&parse(src).unwrap(), &scope, &mut missing)
    }

    #[test]
    fn arithmetic_and_comparison_on_the_model() {
        let model = json!({"player": {"hp": 30, "max": 120, "name": "Ann"}, "orders": [1, 2]});
        assert_eq!(run("player.hp / player.max * 100", &model), json!(25.0));
        assert_eq!(run("1 + 2 * 3", &model), json!(7.0));
        assert_eq!(run("(1 + 2) * 3", &model), json!(9.0));
        assert_eq!(
            run("orders.length > 0 && !player.dead", &model),
            json!(true)
        );
        assert_eq!(run("player.name + '!'", &model), json!("Ann!"));
        assert_eq!(run("player.hp == 30", &model), json!(true));
        assert_eq!(run("-player.hp", &model), json!(-30.0));
    }

    #[test]
    fn a_binding_reads_and_never_does() {
        assert!(parse("player.hp = 3").unwrap_err().contains("only reads"));
        assert!(parse("heal(3)").unwrap_err().contains("calls nothing"));
        assert!(parse("a ? b : c").unwrap_err().contains("no `? :`"));
    }

    #[test]
    fn templates_mix_text_and_values_and_say_what_was_missing() {
        let model = json!({"hp": 7.5, "max": 10});
        let t = Template::parse("HP {{hp}} / {{max}} ({{gone.x}})").unwrap();
        let mut missing = Vec::new();
        let scope = Scope {
            model: &model,
            locals: &[],
        };
        assert_eq!(t.text(&scope, &mut missing), "HP 7.5 / 10 ()");
        assert_eq!(missing, ["gone.x"]);
        let one = Template::parse("{{max}}").unwrap();
        assert_eq!(one.value(&scope, &mut missing), json!(10));
        assert!(!Template::parse("plain").unwrap().is_bound());
    }
}
