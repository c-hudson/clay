//! Expression parser and evaluator for TinyFugue compatibility.
//!
//! Supports TF expression syntax including arithmetic, comparison, string matching,
//! logical operators, and built-in functions.

use super::{TfEngine, TfValue};
use regex::Regex;
use std::collections::HashMap;

/// Token types for the expression lexer
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Integer(i64),
    Float(f64),
    String(String),

    // Identifiers and variables
    Identifier(String),      // Variable name or function name

    // Arithmetic operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,

    // Comparison operators
    Eq,          // == or =
    Ne,          // !=
    Lt,          // <
    Le,          // <=
    Gt,          // >
    Ge,          // >=

    // String match operators
    StrEq,       // =~  (string equality, case-sensitive)
    StrNe,       // !~  (string inequality)
    GlobMatch,   // =/  (glob pattern match)
    GlobNoMatch, // !/  (glob pattern no match)

    // Logical operators
    And,         // &
    Or,          // |
    Not,         // !

    // Assignment
    Assign,      // :=

    // Ternary
    Question,    // ?
    Colon,       // :

    // Increment/decrement
    PlusPlus,    // ++
    MinusMinus,  // --

    // Grouping
    LParen,
    RParen,
    LBrace,      // { for variable substitution
    RBrace,      // }

    // Misc
    Comma,

    // End of expression
    Eof,
}

/// Tokenizer for TF expressions
pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Lexer {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek();
        self.pos += 1;
        c
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    pub fn next_token(&mut self) -> Result<Token, String> {
        self.skip_whitespace();

        let c = match self.peek() {
            Some(c) => c,
            None => return Ok(Token::Eof),
        };

        // Numbers
        if c.is_ascii_digit() || (c == '.' && self.peek_next().is_some_and(|n| n.is_ascii_digit())) {
            return self.read_number();
        }

        // Negative numbers (only if followed by digit)
        if c == '-' && self.peek_next().is_some_and(|n| n.is_ascii_digit()) {
            // Check if this could be subtraction (preceded by operand)
            // For simplicity, we'll handle this in the parser instead
        }

        // Strings
        if c == '"' || c == '\'' || c == '`' {
            return self.read_string(c);
        }

        // Identifiers and keywords
        if c.is_alphabetic() || c == '_' {
            return self.read_identifier();
        }

        // Variable substitution {varname}
        if c == '{' {
            self.advance();
            return Ok(Token::LBrace);
        }
        if c == '}' {
            self.advance();
            return Ok(Token::RBrace);
        }

        // Operators
        self.advance();
        match c {
            '+' => {
                if self.peek() == Some('+') {
                    self.advance();
                    Ok(Token::PlusPlus)
                } else {
                    Ok(Token::Plus)
                }
            }
            '-' => {
                if self.peek() == Some('-') {
                    self.advance();
                    Ok(Token::MinusMinus)
                } else {
                    Ok(Token::Minus)
                }
            }
            '*' => Ok(Token::Star),
            '/' => Ok(Token::Slash),
            '%' => Ok(Token::Percent),
            '=' => {
                match self.peek() {
                    Some('=') => {
                        self.advance();
                        Ok(Token::Eq)
                    }
                    Some('~') => {
                        self.advance();
                        Ok(Token::StrEq)
                    }
                    Some('/') => {
                        self.advance();
                        Ok(Token::GlobMatch)
                    }
                    _ => Ok(Token::Eq)  // Single = is also equality in TF
                }
            }
            '!' => {
                match self.peek() {
                    Some('=') => {
                        self.advance();
                        Ok(Token::Ne)
                    }
                    Some('~') => {
                        self.advance();
                        Ok(Token::StrNe)
                    }
                    Some('/') => {
                        self.advance();
                        Ok(Token::GlobNoMatch)
                    }
                    _ => Ok(Token::Not)
                }
            }
            '<' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token::Le)
                } else {
                    Ok(Token::Lt)
                }
            }
            '>' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token::Ge)
                } else {
                    Ok(Token::Gt)
                }
            }
            '&' => Ok(Token::And),
            '|' => Ok(Token::Or),
            ':' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token::Assign)
                } else {
                    Ok(Token::Colon)
                }
            }
            '?' => Ok(Token::Question),
            '(' => Ok(Token::LParen),
            ')' => Ok(Token::RParen),
            ',' => Ok(Token::Comma),
            _ => Err(format!("Unexpected character: {}", c)),
        }
    }

    fn read_number(&mut self) -> Result<Token, String> {
        let start = self.pos;
        let mut has_dot = false;
        let mut has_exp = false;

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else if c == '.' && !has_dot && !has_exp {
                has_dot = true;
                self.advance();
            } else if (c == 'e' || c == 'E') && !has_exp {
                has_exp = true;
                self.advance();
                // Handle optional sign after exponent
                if self.peek() == Some('+') || self.peek() == Some('-') {
                    self.advance();
                }
            } else {
                break;
            }
        }

        let num_str: String = self.chars[start..self.pos].iter().collect();

        if has_dot || has_exp {
            num_str.parse::<f64>()
                .map(Token::Float)
                .map_err(|e| format!("Invalid float: {}", e))
        } else {
            num_str.parse::<i64>()
                .map(Token::Integer)
                .map_err(|e| format!("Invalid integer: {}", e))
        }
    }

    fn read_string(&mut self, quote: char) -> Result<Token, String> {
        self.advance(); // Skip opening quote
        let mut s = String::new();

        while let Some(c) = self.peek() {
            if c == quote {
                self.advance(); // Skip closing quote
                return Ok(Token::String(s));
            } else if c == '\\' {
                self.advance();
                match self.peek() {
                    Some('n') => { s.push('\n'); self.advance(); }
                    Some('t') => { s.push('\t'); self.advance(); }
                    Some('r') => { s.push('\r'); self.advance(); }
                    Some('\\') => { s.push('\\'); self.advance(); }
                    Some(q) if q == quote => { s.push(q); self.advance(); }
                    Some(c) => { s.push(c); self.advance(); }
                    None => return Err("Unterminated string escape".to_string()),
                }
            } else {
                s.push(c);
                self.advance();
            }
        }

        Err("Unterminated string".to_string())
    }

    fn read_identifier(&mut self) -> Result<Token, String> {
        let start = self.pos;

        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.advance();
            } else {
                break;
            }
        }

        let name: String = self.chars[start..self.pos].iter().collect();
        Ok(Token::Identifier(name))
    }

    /// Tokenize the entire input
    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            if token == Token::Eof {
                tokens.push(token);
                break;
            }
            tokens.push(token);
        }
        Ok(tokens)
    }
}

/// Expression parser using recursive descent with operator precedence
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let token = self.peek().clone();
        self.pos += 1;
        token
    }

    fn expect(&mut self, expected: Token) -> Result<(), String> {
        if self.peek() == &expected {
            self.advance();
            Ok(())
        } else {
            Err(format!("Expected {:?}, got {:?}", expected, self.peek()))
        }
    }

    /// Parse a full expression
    pub fn parse(&mut self) -> Result<Expr, String> {
        self.parse_assignment()
    }

    // Precedence levels (lowest to highest):
    // 1. Assignment (:=)
    // 2. Ternary (?:)
    // 3. Logical OR (|)
    // 4. Logical AND (&)
    // 5. Equality (==, !=, =~, !~, =/, !/)
    // 6. Comparison (<, <=, >, >=)
    // 7. Addition (+, -)
    // 8. Multiplication (*, /, %)
    // 9. Unary (!, -, ++, --)
    // 10. Primary (literals, variables, function calls, parentheses)

    fn parse_assignment(&mut self) -> Result<Expr, String> {
        let expr = self.parse_ternary()?;

        if self.peek() == &Token::Assign {
            self.advance();
            let value = self.parse_assignment()?;  // Right-associative

            // Left side must be an identifier
            if let Expr::Variable(name) = expr {
                return Ok(Expr::Assign(name, Box::new(value)));
            } else {
                return Err("Left side of assignment must be a variable".to_string());
            }
        }

        Ok(expr)
    }

    fn parse_ternary(&mut self) -> Result<Expr, String> {
        let condition = self.parse_or()?;

        if self.peek() == &Token::Question {
            self.advance();

            // Check for omitted true value: expr ? : false_expr
            let true_expr = if self.peek() == &Token::Colon {
                Box::new(condition.clone())
            } else {
                Box::new(self.parse_or()?)
            };

            self.expect(Token::Colon)?;
            let false_expr = Box::new(self.parse_ternary()?);  // Right-associative

            return Ok(Expr::Ternary(Box::new(condition), true_expr, false_expr));
        }

        Ok(condition)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;

        while self.peek() == &Token::Or {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinaryOp(Box::new(left), BinaryOp::Or, Box::new(right));
        }

        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_equality()?;

        while self.peek() == &Token::And {
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::BinaryOp(Box::new(left), BinaryOp::And, Box::new(right));
        }

        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_comparison()?;

        loop {
            let op = match self.peek() {
                Token::Eq => BinaryOp::Eq,
                Token::Ne => BinaryOp::Ne,
                Token::StrEq => BinaryOp::StrEq,
                Token::StrNe => BinaryOp::StrNe,
                Token::GlobMatch => BinaryOp::GlobMatch,
                Token::GlobNoMatch => BinaryOp::GlobNoMatch,
                _ => break,
            };
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::BinaryOp(Box::new(left), op, Box::new(right));
        }

        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_additive()?;

        loop {
            let op = match self.peek() {
                Token::Lt => BinaryOp::Lt,
                Token::Le => BinaryOp::Le,
                Token::Gt => BinaryOp::Gt,
                Token::Ge => BinaryOp::Ge,
                _ => break,
            };
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::BinaryOp(Box::new(left), op, Box::new(right));
        }

        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplicative()?;

        loop {
            let op = match self.peek() {
                Token::Plus => BinaryOp::Add,
                Token::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::BinaryOp(Box::new(left), op, Box::new(right));
        }

        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;

        loop {
            let op = match self.peek() {
                Token::Star => BinaryOp::Mul,
                Token::Slash => BinaryOp::Div,
                Token::Percent => BinaryOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinaryOp(Box::new(left), op, Box::new(right));
        }

        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Token::Not => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::UnaryOp(UnaryOp::Not, Box::new(expr)))
            }
            Token::Minus => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::UnaryOp(UnaryOp::Neg, Box::new(expr)))
            }
            Token::PlusPlus => {
                self.advance();
                if let Token::Identifier(name) = self.advance() {
                    Ok(Expr::PreIncrement(name))
                } else {
                    Err("Expected identifier after ++".to_string())
                }
            }
            Token::MinusMinus => {
                self.advance();
                if let Token::Identifier(name) = self.advance() {
                    Ok(Expr::PreDecrement(name))
                } else {
                    Err("Expected identifier after --".to_string())
                }
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.peek().clone() {
            Token::Integer(n) => {
                self.advance();
                Ok(Expr::Literal(TfValue::Integer(n)))
            }
            Token::Float(f) => {
                self.advance();
                Ok(Expr::Literal(TfValue::Float(f)))
            }
            Token::String(s) => {
                self.advance();
                Ok(Expr::Literal(TfValue::String(s)))
            }
            Token::Identifier(name) => {
                self.advance();
                // Check if it's a function call
                if self.peek() == &Token::LParen {
                    self.advance();
                    let args = self.parse_args()?;
                    self.expect(Token::RParen)?;
                    Ok(Expr::FunctionCall(name, args))
                } else {
                    Ok(Expr::Variable(name))
                }
            }
            Token::LBrace => {
                // {varname} variable substitution
                self.advance();
                if let Token::Identifier(name) = self.advance() {
                    self.expect(Token::RBrace)?;
                    Ok(Expr::Variable(name))
                } else {
                    Err("Expected identifier in {}".to_string())
                }
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse()?;
                self.expect(Token::RParen)?;
                Ok(expr)
            }
            _ => Err(format!("Unexpected token: {:?}", self.peek())),
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();

        if self.peek() == &Token::RParen {
            return Ok(args);
        }

        args.push(self.parse()?);

        while self.peek() == &Token::Comma {
            self.advance();
            args.push(self.parse()?);
        }

        Ok(args)
    }
}

/// Binary operators
#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOp {
    // Arithmetic
    Add, Sub, Mul, Div, Mod,
    // Comparison
    Eq, Ne, Lt, Le, Gt, Ge,
    // String matching
    StrEq, StrNe, GlobMatch, GlobNoMatch,
    // Logical
    And, Or,
}

/// Unary operators
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Not,
    Neg,
}

/// Expression AST
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(TfValue),
    Variable(String),
    BinaryOp(Box<Expr>, BinaryOp, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Assign(String, Box<Expr>),
    PreIncrement(String),
    PreDecrement(String),
    FunctionCall(String, Vec<Expr>),
}

/// Expression evaluator
pub struct Evaluator<'a> {
    engine: &'a mut TfEngine,
    /// Cache for compiled regexes
    regex_cache: HashMap<String, Regex>,
}

impl<'a> Evaluator<'a> {
    pub fn new(engine: &'a mut TfEngine) -> Self {
        Evaluator {
            engine,
            regex_cache: HashMap::new(),
        }
    }

    /// Evaluate an expression and return the result
    pub fn eval(&mut self, expr: &Expr) -> Result<TfValue, String> {
        match expr {
            Expr::Literal(val) => Ok(val.clone()),

            Expr::Variable(name) => {
                self.engine.get_var(name)
                    .cloned()
                    .ok_or_else(|| format!("Undefined variable: {}", name))
            }

            Expr::BinaryOp(left, op, right) => {
                self.eval_binary_op(left, op, right)
            }

            Expr::UnaryOp(op, expr) => {
                let val = self.eval(expr)?;
                match op {
                    UnaryOp::Not => Ok(TfValue::Integer(if val.to_bool() { 0 } else { 1 })),
                    UnaryOp::Neg => {
                        match val {
                            TfValue::Integer(n) => Ok(TfValue::Integer(-n)),
                            TfValue::Float(f) => Ok(TfValue::Float(-f)),
                            TfValue::String(s) => {
                                if let Ok(n) = s.parse::<i64>() {
                                    Ok(TfValue::Integer(-n))
                                } else if let Ok(f) = s.parse::<f64>() {
                                    Ok(TfValue::Float(-f))
                                } else {
                                    Err(format!("Cannot negate string: {}", s))
                                }
                            }
                        }
                    }
                }
            }

            Expr::Ternary(cond, true_expr, false_expr) => {
                let cond_val = self.eval(cond)?;
                if cond_val.to_bool() {
                    self.eval(true_expr)
                } else {
                    self.eval(false_expr)
                }
            }

            Expr::Assign(name, value) => {
                let val = self.eval(value)?;
                self.engine.set_local(name, val.clone());
                Ok(val)
            }

            Expr::PreIncrement(name) => {
                let val = self.engine.get_var(name)
                    .cloned()
                    .unwrap_or(TfValue::Integer(0));
                let new_val = match val {
                    TfValue::Integer(n) => TfValue::Integer(n + 1),
                    TfValue::Float(f) => TfValue::Float(f + 1.0),
                    TfValue::String(s) => {
                        if let Ok(n) = s.parse::<i64>() {
                            TfValue::Integer(n + 1)
                        } else {
                            TfValue::Integer(1)
                        }
                    }
                };
                self.engine.set_local(name, new_val.clone());
                Ok(new_val)
            }

            Expr::PreDecrement(name) => {
                let val = self.engine.get_var(name)
                    .cloned()
                    .unwrap_or(TfValue::Integer(0));
                let new_val = match val {
                    TfValue::Integer(n) => TfValue::Integer(n - 1),
                    TfValue::Float(f) => TfValue::Float(f - 1.0),
                    TfValue::String(s) => {
                        if let Ok(n) = s.parse::<i64>() {
                            TfValue::Integer(n - 1)
                        } else {
                            TfValue::Integer(-1)
                        }
                    }
                };
                self.engine.set_local(name, new_val.clone());
                Ok(new_val)
            }

            Expr::FunctionCall(name, args) => {
                self.eval_function(name, args)
            }
        }
    }

    fn eval_binary_op(&mut self, left: &Expr, op: &BinaryOp, right: &Expr) -> Result<TfValue, String> {
        // Short-circuit for logical operators
        if *op == BinaryOp::And {
            let left_val = self.eval(left)?;
            if !left_val.to_bool() {
                return Ok(TfValue::Integer(0));
            }
            let right_val = self.eval(right)?;
            return Ok(TfValue::Integer(if right_val.to_bool() { 1 } else { 0 }));
        }

        if *op == BinaryOp::Or {
            let left_val = self.eval(left)?;
            if left_val.to_bool() {
                return Ok(TfValue::Integer(1));
            }
            let right_val = self.eval(right)?;
            return Ok(TfValue::Integer(if right_val.to_bool() { 1 } else { 0 }));
        }

        let left_val = self.eval(left)?;
        let right_val = self.eval(right)?;

        match op {
            // Arithmetic
            BinaryOp::Add => self.eval_arithmetic(&left_val, &right_val, |a, b| a + b, |a, b| a + b),
            BinaryOp::Sub => self.eval_arithmetic(&left_val, &right_val, |a, b| a - b, |a, b| a - b),
            BinaryOp::Mul => self.eval_arithmetic(&left_val, &right_val, |a, b| a * b, |a, b| a * b),
            BinaryOp::Div => {
                // Check for division by zero
                let right_num = right_val.to_float().unwrap_or(0.0);
                if right_num == 0.0 {
                    return Err("Division by zero".to_string());
                }
                self.eval_arithmetic(&left_val, &right_val, |a, b| a / b, |a, b| a / b)
            }
            BinaryOp::Mod => {
                let left_int = left_val.to_int().unwrap_or(0);
                let right_int = right_val.to_int().unwrap_or(1);
                if right_int == 0 {
                    return Err("Modulo by zero".to_string());
                }
                Ok(TfValue::Integer(left_int % right_int))
            }

            // Numeric comparison
            BinaryOp::Eq => {
                let result = self.compare_values(&left_val, &right_val) == 0;
                Ok(TfValue::Integer(if result { 1 } else { 0 }))
            }
            BinaryOp::Ne => {
                let result = self.compare_values(&left_val, &right_val) != 0;
                Ok(TfValue::Integer(if result { 1 } else { 0 }))
            }
            BinaryOp::Lt => {
                let result = self.compare_values(&left_val, &right_val) < 0;
                Ok(TfValue::Integer(if result { 1 } else { 0 }))
            }
            BinaryOp::Le => {
                let result = self.compare_values(&left_val, &right_val) <= 0;
                Ok(TfValue::Integer(if result { 1 } else { 0 }))
            }
            BinaryOp::Gt => {
                let result = self.compare_values(&left_val, &right_val) > 0;
                Ok(TfValue::Integer(if result { 1 } else { 0 }))
            }
            BinaryOp::Ge => {
                let result = self.compare_values(&left_val, &right_val) >= 0;
                Ok(TfValue::Integer(if result { 1 } else { 0 }))
            }

            // String comparison (case-sensitive)
            BinaryOp::StrEq => {
                let result = left_val.to_string_value() == right_val.to_string_value();
                Ok(TfValue::Integer(if result { 1 } else { 0 }))
            }
            BinaryOp::StrNe => {
                let result = left_val.to_string_value() != right_val.to_string_value();
                Ok(TfValue::Integer(if result { 1 } else { 0 }))
            }

            // Glob pattern matching
            BinaryOp::GlobMatch => {
                let text = left_val.to_string_value();
                let pattern = right_val.to_string_value();
                let result = self.glob_match(&text, &pattern);
                Ok(TfValue::Integer(if result { 1 } else { 0 }))
            }
            BinaryOp::GlobNoMatch => {
                let text = left_val.to_string_value();
                let pattern = right_val.to_string_value();
                let result = !self.glob_match(&text, &pattern);
                Ok(TfValue::Integer(if result { 1 } else { 0 }))
            }

            // Already handled above
            BinaryOp::And | BinaryOp::Or => unreachable!(),
        }
    }

    fn eval_arithmetic<F, G>(&self, left: &TfValue, right: &TfValue, int_op: F, float_op: G) -> Result<TfValue, String>
    where
        F: Fn(i64, i64) -> i64,
        G: Fn(f64, f64) -> f64,
    {
        // If either is a float, use float arithmetic
        match (left, right) {
            (TfValue::Float(a), TfValue::Float(b)) => Ok(TfValue::Float(float_op(*a, *b))),
            (TfValue::Float(a), _) => {
                let b = right.to_float().unwrap_or(0.0);
                Ok(TfValue::Float(float_op(*a, b)))
            }
            (_, TfValue::Float(b)) => {
                let a = left.to_float().unwrap_or(0.0);
                Ok(TfValue::Float(float_op(a, *b)))
            }
            _ => {
                let a = left.to_int().unwrap_or(0);
                let b = right.to_int().unwrap_or(0);
                Ok(TfValue::Integer(int_op(a, b)))
            }
        }
    }

    fn compare_values(&self, left: &TfValue, right: &TfValue) -> i32 {
        // Try numeric comparison first
        match (left.to_float(), right.to_float()) {
            (Some(a), Some(b)) => {
                if a < b { -1 }
                else if a > b { 1 }
                else { 0 }
            }
            _ => {
                // Fall back to string comparison
                let a = left.to_string_value();
                let b = right.to_string_value();
                a.cmp(&b) as i32
            }
        }
    }

    fn glob_match(&mut self, text: &str, pattern: &str) -> bool {
        // Convert glob to regex
        let regex_pattern = glob_to_regex(pattern);

        // Get or compile regex
        let regex = self.regex_cache.entry(regex_pattern.clone())
            .or_insert_with(|| {
                Regex::new(&regex_pattern).unwrap_or_else(|_| Regex::new("^$").unwrap())
            });

        regex.is_match(text)
    }

    fn eval_function(&mut self, name: &str, args: &[Expr]) -> Result<TfValue, String> {
        match name.to_lowercase().as_str() {
            "strlen" => {
                if args.len() != 1 {
                    return Err("strlen requires 1 argument".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                Ok(TfValue::Integer(s.len() as i64))
            }

            "substr" => {
                if args.len() < 2 || args.len() > 3 {
                    return Err("substr requires 2 or 3 arguments".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                let start = self.eval(&args[1])?.to_int().unwrap_or(0) as usize;
                let len = if args.len() == 3 {
                    self.eval(&args[2])?.to_int().unwrap_or(s.len() as i64) as usize
                } else {
                    s.len()
                };

                let chars: Vec<char> = s.chars().collect();
                let start = start.min(chars.len());
                let end = (start + len).min(chars.len());
                let result: String = chars[start..end].iter().collect();
                Ok(TfValue::String(result))
            }

            "strcat" => {
                let mut result = String::new();
                for arg in args {
                    result.push_str(&self.eval(arg)?.to_string_value());
                }
                Ok(TfValue::String(result))
            }

            "strcmp" => {
                if args.len() != 2 {
                    return Err("strcmp requires 2 arguments".to_string());
                }
                let a = self.eval(&args[0])?.to_string_value();
                let b = self.eval(&args[1])?.to_string_value();
                Ok(TfValue::Integer(a.cmp(&b) as i64))
            }

            "tolower" => {
                if args.len() != 1 {
                    return Err("tolower requires 1 argument".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                Ok(TfValue::String(s.to_lowercase()))
            }

            "toupper" => {
                if args.len() != 1 {
                    return Err("toupper requires 1 argument".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                Ok(TfValue::String(s.to_uppercase()))
            }

            "time" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                Ok(TfValue::Integer(now))
            }

            "rand" => {
                if args.is_empty() {
                    // Random float between 0 and 1
                    let r = simple_random() as f64 / u32::MAX as f64;
                    Ok(TfValue::Float(r))
                } else {
                    // Random integer between 0 and max-1
                    let max = self.eval(&args[0])?.to_int().unwrap_or(100);
                    if max <= 0 {
                        return Ok(TfValue::Integer(0));
                    }
                    let r = (simple_random() as i64) % max;
                    Ok(TfValue::Integer(r.abs()))
                }
            }

            "abs" => {
                if args.len() != 1 {
                    return Err("abs requires 1 argument".to_string());
                }
                let val = self.eval(&args[0])?;
                match val {
                    TfValue::Integer(n) => Ok(TfValue::Integer(n.abs())),
                    TfValue::Float(f) => Ok(TfValue::Float(f.abs())),
                    TfValue::String(s) => {
                        if let Ok(n) = s.parse::<i64>() {
                            Ok(TfValue::Integer(n.abs()))
                        } else if let Ok(f) = s.parse::<f64>() {
                            Ok(TfValue::Float(f.abs()))
                        } else {
                            Ok(TfValue::Integer(0))
                        }
                    }
                }
            }

            "min" => {
                if args.len() < 2 {
                    return Err("min requires at least 2 arguments".to_string());
                }
                let mut result = self.eval(&args[0])?;
                for arg in &args[1..] {
                    let val = self.eval(arg)?;
                    if self.compare_values(&val, &result) < 0 {
                        result = val;
                    }
                }
                Ok(result)
            }

            "max" => {
                if args.len() < 2 {
                    return Err("max requires at least 2 arguments".to_string());
                }
                let mut result = self.eval(&args[0])?;
                for arg in &args[1..] {
                    let val = self.eval(arg)?;
                    if self.compare_values(&val, &result) > 0 {
                        result = val;
                    }
                }
                Ok(result)
            }

            "ascii" => {
                if args.len() != 1 {
                    return Err("ascii requires 1 argument".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                let code = s.chars().next().map(|c| c as i64).unwrap_or(0);
                Ok(TfValue::Integer(code))
            }

            "char" => {
                if args.len() != 1 {
                    return Err("char requires 1 argument".to_string());
                }
                let code = self.eval(&args[0])?.to_int().unwrap_or(0) as u32;
                let c = char::from_u32(code).unwrap_or('\0');
                Ok(TfValue::String(c.to_string()))
            }

            _ => Err(format!("Unknown function: {}", name)),
        }
    }
}

/// Convert a glob pattern to a regex pattern
fn glob_to_regex(pattern: &str) -> String {
    let mut result = String::from("^");

    for c in pattern.chars() {
        match c {
            '*' => result.push_str(".*"),
            '?' => result.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }

    result.push('$');
    result
}

/// Simple random number generator (xorshift32)
pub fn simple_random() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    let mut x = SEED.load(std::sync::atomic::Ordering::Relaxed);
    if x == 0 {
        x = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u32)
            .unwrap_or(12345);
    }

    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;

    SEED.store(x, std::sync::atomic::Ordering::Relaxed);
    x
}

/// Parse and evaluate an expression string
pub fn evaluate(engine: &mut TfEngine, expr_str: &str) -> Result<TfValue, String> {
    let mut lexer = Lexer::new(expr_str);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    let ast = parser.parse()?;
    let mut evaluator = Evaluator::new(engine);
    evaluator.eval(&ast)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexer_numbers() {
        let mut lexer = Lexer::new("42 3.14 1e10");
        assert_eq!(lexer.next_token().unwrap(), Token::Integer(42));
        assert_eq!(lexer.next_token().unwrap(), Token::Float(3.14));
        assert!(matches!(lexer.next_token().unwrap(), Token::Float(_)));
    }

    #[test]
    fn test_lexer_strings() {
        let mut lexer = Lexer::new(r#""hello" 'world'"#);
        assert_eq!(lexer.next_token().unwrap(), Token::String("hello".to_string()));
        assert_eq!(lexer.next_token().unwrap(), Token::String("world".to_string()));
    }

    #[test]
    fn test_lexer_operators() {
        let mut lexer = Lexer::new("+ - * / == != <= >= =~ !~ =/ !/ & | := ? :");
        assert_eq!(lexer.next_token().unwrap(), Token::Plus);
        assert_eq!(lexer.next_token().unwrap(), Token::Minus);
        assert_eq!(lexer.next_token().unwrap(), Token::Star);
        assert_eq!(lexer.next_token().unwrap(), Token::Slash);
        assert_eq!(lexer.next_token().unwrap(), Token::Eq);
        assert_eq!(lexer.next_token().unwrap(), Token::Ne);
        assert_eq!(lexer.next_token().unwrap(), Token::Le);
        assert_eq!(lexer.next_token().unwrap(), Token::Ge);
        assert_eq!(lexer.next_token().unwrap(), Token::StrEq);
        assert_eq!(lexer.next_token().unwrap(), Token::StrNe);
        assert_eq!(lexer.next_token().unwrap(), Token::GlobMatch);
        assert_eq!(lexer.next_token().unwrap(), Token::GlobNoMatch);
        assert_eq!(lexer.next_token().unwrap(), Token::And);
        assert_eq!(lexer.next_token().unwrap(), Token::Or);
        assert_eq!(lexer.next_token().unwrap(), Token::Assign);
        assert_eq!(lexer.next_token().unwrap(), Token::Question);
        assert_eq!(lexer.next_token().unwrap(), Token::Colon);
    }

    #[test]
    fn test_eval_arithmetic() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "2 + 3").unwrap(), TfValue::Integer(5));
        assert_eq!(evaluate(&mut engine, "10 - 4").unwrap(), TfValue::Integer(6));
        assert_eq!(evaluate(&mut engine, "3 * 4").unwrap(), TfValue::Integer(12));
        assert_eq!(evaluate(&mut engine, "15 / 3").unwrap(), TfValue::Integer(5));
        assert_eq!(evaluate(&mut engine, "17 % 5").unwrap(), TfValue::Integer(2));
    }

    #[test]
    fn test_eval_precedence() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "2 + 3 * 4").unwrap(), TfValue::Integer(14));
        assert_eq!(evaluate(&mut engine, "(2 + 3) * 4").unwrap(), TfValue::Integer(20));
    }

    #[test]
    fn test_eval_comparison() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "5 > 3").unwrap(), TfValue::Integer(1));
        assert_eq!(evaluate(&mut engine, "5 < 3").unwrap(), TfValue::Integer(0));
        assert_eq!(evaluate(&mut engine, "5 == 5").unwrap(), TfValue::Integer(1));
        assert_eq!(evaluate(&mut engine, "5 != 3").unwrap(), TfValue::Integer(1));
    }

    #[test]
    fn test_eval_logical() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "1 & 1").unwrap(), TfValue::Integer(1));
        assert_eq!(evaluate(&mut engine, "1 & 0").unwrap(), TfValue::Integer(0));
        assert_eq!(evaluate(&mut engine, "1 | 0").unwrap(), TfValue::Integer(1));
        assert_eq!(evaluate(&mut engine, "0 | 0").unwrap(), TfValue::Integer(0));
        assert_eq!(evaluate(&mut engine, "!0").unwrap(), TfValue::Integer(1));
        assert_eq!(evaluate(&mut engine, "!1").unwrap(), TfValue::Integer(0));
    }

    #[test]
    fn test_eval_ternary() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "1 ? 10 : 20").unwrap(), TfValue::Integer(10));
        assert_eq!(evaluate(&mut engine, "0 ? 10 : 20").unwrap(), TfValue::Integer(20));
    }

    #[test]
    fn test_eval_variables() {
        let mut engine = TfEngine::new();
        engine.set_global("x", TfValue::Integer(5));
        assert_eq!(evaluate(&mut engine, "x + 3").unwrap(), TfValue::Integer(8));
        assert_eq!(evaluate(&mut engine, "{x} * 2").unwrap(), TfValue::Integer(10));
    }

    #[test]
    fn test_eval_assignment() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "x := 10").unwrap(), TfValue::Integer(10));
        assert_eq!(engine.get_var("x").map(|v| v.to_int()), Some(Some(10)));
    }

    #[test]
    fn test_eval_functions() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "strlen(\"hello\")").unwrap(), TfValue::Integer(5));
        assert_eq!(evaluate(&mut engine, "toupper(\"hello\")").unwrap(), TfValue::String("HELLO".to_string()));
        assert_eq!(evaluate(&mut engine, "tolower(\"WORLD\")").unwrap(), TfValue::String("world".to_string()));
        assert_eq!(evaluate(&mut engine, "abs(-5)").unwrap(), TfValue::Integer(5));
        assert_eq!(evaluate(&mut engine, "min(3, 1, 4)").unwrap(), TfValue::Integer(1));
        assert_eq!(evaluate(&mut engine, "max(3, 1, 4)").unwrap(), TfValue::Integer(4));
    }

    #[test]
    fn test_eval_string_match() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, r#""hello" =~ "hello""#).unwrap(), TfValue::Integer(1));
        assert_eq!(evaluate(&mut engine, r#""hello" =~ "world""#).unwrap(), TfValue::Integer(0));
        assert_eq!(evaluate(&mut engine, r#""hello" =/ "hel*""#).unwrap(), TfValue::Integer(1));
        assert_eq!(evaluate(&mut engine, r#""hello" =/ "wor*""#).unwrap(), TfValue::Integer(0));
    }

    #[test]
    fn test_glob_to_regex() {
        assert_eq!(glob_to_regex("hello"), "^hello$");
        assert_eq!(glob_to_regex("hel*"), "^hel.*$");
        assert_eq!(glob_to_regex("h?llo"), "^h.llo$");
        assert_eq!(glob_to_regex("test.txt"), r"^test\.txt$");
    }
}
