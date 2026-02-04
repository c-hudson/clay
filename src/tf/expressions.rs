//! Expression parser and evaluator for TinyFugue compatibility.
//!
//! Supports TF expression syntax including arithmetic, comparison, string matching,
//! logical operators, and built-in functions.

use super::{TfEngine, TfValue};
use regex::Regex;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};

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
                // {varname}, {*}, {n}, or {-n} substitution
                self.advance();

                // Check what's inside the braces
                let is_star = matches!(self.peek(), Token::Star);
                let is_minus = matches!(self.peek(), Token::Minus);
                let integer_val = if let Token::Integer(n) = self.peek() { Some(*n) } else { None };
                let ident_val = if let Token::Identifier(s) = self.peek() { Some(s.clone()) } else { None };

                if is_star {
                    // {*} - all arguments
                    self.advance();
                    self.expect(Token::RBrace)?;
                    Ok(Expr::Variable("*".to_string()))
                } else if is_minus {
                    // {-n} - argument from end
                    self.advance();
                    if let Token::Integer(n) = self.advance() {
                        self.expect(Token::RBrace)?;
                        Ok(Expr::Variable(format!("-{}", n)))
                    } else {
                        Err("Expected number after - in {-n}".to_string())
                    }
                } else if let Some(n) = integer_val {
                    // {n} or {n-} - positional argument or range to end
                    self.advance();
                    // Check for {n-} pattern (args from n to end)
                    if matches!(self.peek(), Token::Minus) {
                        self.advance();
                        self.expect(Token::RBrace)?;
                        // Return special variable name for "n to end" range
                        Ok(Expr::Variable(format!("{}-", n)))
                    } else {
                        self.expect(Token::RBrace)?;
                        Ok(Expr::Variable(n.to_string()))
                    }
                } else if let Some(name) = ident_val {
                    // {varname} - variable
                    self.advance();
                    self.expect(Token::RBrace)?;
                    Ok(Expr::Variable(name))
                } else {
                    Err(format!("Expected identifier, *, or number in {{}}, got {:?}", self.peek()))
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
                // Handle special variable names for argument access
                if name.ends_with('-') && name.len() > 1 {
                    // {n-} - arguments from position n to end, joined with spaces
                    if let Ok(start) = name[..name.len()-1].parse::<usize>() {
                        let argc = self.engine.get_var("#")
                            .and_then(|v| v.to_int())
                            .unwrap_or(0) as usize;
                        if start > 0 && start <= argc {
                            // Collect args from start to end
                            let mut parts = Vec::new();
                            for i in start..=argc {
                                if let Some(val) = self.engine.get_var(&i.to_string()) {
                                    parts.push(val.to_string_value());
                                }
                            }
                            return Ok(TfValue::String(parts.join(" ")));
                        }
                    }
                    return Ok(TfValue::String(String::new()));
                } else if name.starts_with('-') {
                    // {-n} - argument from end (1-indexed from end)
                    if let Ok(n) = name[1..].parse::<usize>() {
                        // Get total argument count
                        let argc = self.engine.get_var("#")
                            .and_then(|v| v.to_int())
                            .unwrap_or(0) as usize;
                        if n > 0 && n <= argc {
                            // -1 = last arg, -2 = second to last, etc.
                            let idx = argc - n + 1;
                            return self.engine.get_var(&idx.to_string())
                                .cloned()
                                .ok_or_else(|| format!("Argument {} not found", idx));
                        }
                    }
                    return Ok(TfValue::String(String::new()));
                }
                // Return empty string for undefined variables (TF behavior)
                Ok(self.engine.get_var(name)
                    .cloned()
                    .unwrap_or_else(|| TfValue::String(String::new())))
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
                let start_val = self.eval(&args[1])?.to_int().unwrap_or(0);
                let len_val = if args.len() == 3 {
                    self.eval(&args[2])?.to_int().unwrap_or(s.len() as i64)
                } else {
                    s.len() as i64
                };

                // Handle negative values - treat as 0
                let start = if start_val < 0 { 0usize } else { start_val as usize };
                let len = if len_val < 0 { 0usize } else { len_val as usize };

                let chars: Vec<char> = s.chars().collect();
                let start = start.min(chars.len());
                let end = start.saturating_add(len).min(chars.len());
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

            "strncmp" => {
                if args.len() != 3 {
                    return Err("strncmp requires 3 arguments".to_string());
                }
                let a = self.eval(&args[0])?.to_string_value();
                let b = self.eval(&args[1])?.to_string_value();
                let n = self.eval(&args[2])?.to_int().unwrap_or(0) as usize;
                let a_prefix: String = a.chars().take(n).collect();
                let b_prefix: String = b.chars().take(n).collect();
                Ok(TfValue::Integer(a_prefix.cmp(&b_prefix) as i64))
            }

            "strchr" => {
                if args.len() != 2 {
                    return Err("strchr requires 2 arguments".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                let chars = self.eval(&args[1])?.to_string_value();
                let pos = s.chars().position(|c| chars.contains(c));
                Ok(TfValue::Integer(pos.map(|p| p as i64).unwrap_or(-1)))
            }

            "strrchr" => {
                if args.len() != 2 {
                    return Err("strrchr requires 2 arguments".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                let chars = self.eval(&args[1])?.to_string_value();
                let pos = s.chars().collect::<Vec<_>>().iter().rposition(|c| chars.contains(*c));
                Ok(TfValue::Integer(pos.map(|p| p as i64).unwrap_or(-1)))
            }

            "strrep" => {
                if args.len() != 2 {
                    return Err("strrep requires 2 arguments".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                let n = self.eval(&args[1])?.to_int().unwrap_or(0);
                if n <= 0 {
                    Ok(TfValue::String(String::new()))
                } else {
                    Ok(TfValue::String(s.repeat(n as usize)))
                }
            }

            "pad" => {
                // pad([s, i]...) - pad strings to specified widths
                if args.len() % 2 != 0 {
                    return Err("pad requires pairs of (string, width) arguments".to_string());
                }
                let mut result = String::new();
                for i in (0..args.len()).step_by(2) {
                    let s = self.eval(&args[i])?.to_string_value();
                    let width = self.eval(&args[i + 1])?.to_int().unwrap_or(0);
                    let abs_width = width.unsigned_abs() as usize;
                    if width >= 0 {
                        // Right-justify (left-pad)
                        result.push_str(&format!("{:>width$}", s, width = abs_width));
                    } else {
                        // Left-justify (right-pad)
                        result.push_str(&format!("{:<width$}", s, width = abs_width));
                    }
                }
                Ok(TfValue::String(result))
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
                    // rand() - random integer in system range
                    Ok(TfValue::Integer(simple_random() as i64))
                } else if args.len() == 1 {
                    // rand(max) - random integer in [0, max-1]
                    let max = self.eval(&args[0])?.to_int().unwrap_or(100);
                    if max <= 0 {
                        return Ok(TfValue::Integer(0));
                    }
                    let r = (simple_random() as i64) % max;
                    Ok(TfValue::Integer(r.abs()))
                } else {
                    // rand(min, max) - random integer in [min, max]
                    let min = self.eval(&args[0])?.to_int().unwrap_or(0);
                    let max = self.eval(&args[1])?.to_int().unwrap_or(100);
                    if max < min {
                        return Ok(TfValue::Integer(min));
                    }
                    let range = (max - min + 1) as u64;
                    let r = min + ((simple_random() as u64) % range) as i64;
                    Ok(TfValue::Integer(r))
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

            // Trigonometric functions
            "sin" => {
                if args.len() != 1 {
                    return Err("sin requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                Ok(TfValue::Float(x.sin()))
            }

            "cos" => {
                if args.len() != 1 {
                    return Err("cos requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                Ok(TfValue::Float(x.cos()))
            }

            "tan" => {
                if args.len() != 1 {
                    return Err("tan requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                Ok(TfValue::Float(x.tan()))
            }

            "asin" => {
                if args.len() != 1 {
                    return Err("asin requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                if !(-1.0..=1.0).contains(&x) {
                    return Err("asin: argument must be in [-1, 1]".to_string());
                }
                Ok(TfValue::Float(x.asin()))
            }

            "acos" => {
                if args.len() != 1 {
                    return Err("acos requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                if !(-1.0..=1.0).contains(&x) {
                    return Err("acos: argument must be in [-1, 1]".to_string());
                }
                Ok(TfValue::Float(x.acos()))
            }

            "atan" => {
                if args.len() != 1 {
                    return Err("atan requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                Ok(TfValue::Float(x.atan()))
            }

            "exp" => {
                if args.len() != 1 {
                    return Err("exp requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                Ok(TfValue::Float(x.exp()))
            }

            "pow" => {
                if args.len() != 2 {
                    return Err("pow requires 2 arguments".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                let y = self.eval(&args[1])?.to_float().unwrap_or(0.0);
                Ok(TfValue::Float(x.powf(y)))
            }

            "sqrt" => {
                if args.len() != 1 {
                    return Err("sqrt requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                if x < 0.0 {
                    return Err("sqrt: argument must be non-negative".to_string());
                }
                Ok(TfValue::Float(x.sqrt()))
            }

            "log" => {
                if args.len() != 1 {
                    return Err("log requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                if x <= 0.0 {
                    return Err("log: argument must be positive".to_string());
                }
                Ok(TfValue::Float(x.ln()))
            }

            "log10" => {
                if args.len() != 1 {
                    return Err("log10 requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                if x <= 0.0 {
                    return Err("log10: argument must be positive".to_string());
                }
                Ok(TfValue::Float(x.log10()))
            }

            "mod" => {
                if args.len() != 2 {
                    return Err("mod requires 2 arguments".to_string());
                }
                let i = self.eval(&args[0])?.to_int().unwrap_or(0);
                let j = self.eval(&args[1])?.to_int().unwrap_or(1);
                if j == 0 {
                    return Err("mod: division by zero".to_string());
                }
                Ok(TfValue::Integer(i % j))
            }

            "trunc" => {
                if args.len() != 1 {
                    return Err("trunc requires 1 argument".to_string());
                }
                let x = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                Ok(TfValue::Integer(x.trunc() as i64))
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

            "addworld" => {
                // addworld(name, type, [host, port [, char, pass [, file [, flags]]]])
                // Minimum 1 argument (name), type is optional and ignored (defaults to MUD)
                if args.is_empty() {
                    return Err("addworld requires at least 1 argument (name)".to_string());
                }

                let name = self.eval(&args[0])?.to_string_value();
                if name.is_empty() {
                    return Err("addworld: name cannot be empty".to_string());
                }
                if name.contains(' ') {
                    return Err("addworld: name cannot contain spaces".to_string());
                }
                if name.starts_with('(') {
                    return Err("addworld: name cannot start with '('".to_string());
                }

                // Type is ignored (arg index 1) - we default to MUD
                let host = if args.len() > 2 {
                    let h = self.eval(&args[2])?.to_string_value();
                    if h.is_empty() { None } else { Some(h) }
                } else {
                    None
                };

                let port = if args.len() > 3 {
                    let p = self.eval(&args[3])?.to_string_value();
                    if p.is_empty() { None } else { Some(p) }
                } else {
                    None
                };

                let user = if args.len() > 4 {
                    let u = self.eval(&args[4])?.to_string_value();
                    if u.is_empty() { None } else { Some(u) }
                } else {
                    None
                };

                let password = if args.len() > 5 {
                    let p = self.eval(&args[5])?.to_string_value();
                    if p.is_empty() { None } else { Some(p) }
                } else {
                    None
                };

                // file (arg 6) is ignored
                // flags (arg 7) - check for 'x' (SSL)
                let use_ssl = if args.len() > 7 {
                    let flags = self.eval(&args[7])?.to_string_value();
                    flags.contains('x')
                } else {
                    false
                };

                // Queue the world operation for the main app to process
                self.engine.pending_world_ops.push(super::PendingWorldOp {
                    name: name.clone(),
                    host,
                    port,
                    user,
                    password,
                    use_ssl,
                });

                // Return 1 for success (TF convention)
                Ok(TfValue::Integer(1))
            }

            // regmatch(pattern, string) - regex matching with capture groups
            "regmatch" => {
                if args.len() != 2 {
                    return Err("regmatch requires 2 arguments (pattern, string)".to_string());
                }
                let pattern = self.eval(&args[0])?.to_string_value();
                let text = self.eval(&args[1])?.to_string_value();

                // Clear previous captures
                self.engine.regex_captures.clear();

                // Compile regex
                let regex = match Regex::new(&pattern) {
                    Ok(r) => r,
                    Err(e) => return Err(format!("Invalid regex: {}", e)),
                };

                // Try to match
                if let Some(caps) = regex.captures(&text) {
                    // Store captures in P0-P9
                    for i in 0..10 {
                        if let Some(m) = caps.get(i) {
                            self.engine.regex_captures.push(m.as_str().to_string());
                        } else {
                            self.engine.regex_captures.push(String::new());
                        }
                    }
                    Ok(TfValue::Integer(1))
                } else {
                    // No match - clear captures
                    for _ in 0..10 {
                        self.engine.regex_captures.push(String::new());
                    }
                    Ok(TfValue::Integer(0))
                }
            }

            // replace(str, old, new [, count]) - string replacement
            "replace" => {
                if args.len() < 3 || args.len() > 4 {
                    return Err("replace requires 3 or 4 arguments (str, old, new [, count])".to_string());
                }
                let text = self.eval(&args[0])?.to_string_value();
                let old = self.eval(&args[1])?.to_string_value();
                let new = self.eval(&args[2])?.to_string_value();

                if old.is_empty() {
                    return Ok(TfValue::String(text));
                }

                let result = if args.len() == 4 {
                    let count = self.eval(&args[3])?.to_int().unwrap_or(0) as usize;
                    if count == 0 {
                        text.replace(&old, &new)
                    } else {
                        text.replacen(&old, &new, count)
                    }
                } else {
                    text.replace(&old, &new)
                };

                Ok(TfValue::String(result))
            }

            // strstr(str, substr) - find position of substring (0-indexed, -1 if not found)
            "strstr" => {
                if args.len() != 2 {
                    return Err("strstr requires 2 arguments (str, substr)".to_string());
                }
                let text = self.eval(&args[0])?.to_string_value();
                let substr = self.eval(&args[1])?.to_string_value();

                let pos = text.find(&substr).map(|p| p as i64).unwrap_or(-1);
                Ok(TfValue::Integer(pos))
            }

            // sprintf(format, args...) - formatted string
            "sprintf" => {
                if args.is_empty() {
                    return Err("sprintf requires at least 1 argument (format)".to_string());
                }
                let format = self.eval(&args[0])?.to_string_value();

                // Evaluate all arguments first
                let mut arg_values: Vec<TfValue> = Vec::new();
                for arg in &args[1..] {
                    arg_values.push(self.eval(arg)?);
                }

                // Simple sprintf implementation supporting %s, %d, %i, %f, %%
                let mut result = String::new();
                let mut arg_idx = 0;
                let mut chars = format.chars().peekable();

                while let Some(c) = chars.next() {
                    if c == '%' {
                        match chars.peek() {
                            Some('%') => {
                                chars.next();
                                result.push('%');
                            }
                            Some('s') => {
                                chars.next();
                                if arg_idx < arg_values.len() {
                                    result.push_str(&arg_values[arg_idx].to_string_value());
                                    arg_idx += 1;
                                }
                            }
                            Some('d') | Some('i') => {
                                chars.next();
                                if arg_idx < arg_values.len() {
                                    let val = arg_values[arg_idx].to_int().unwrap_or(0);
                                    result.push_str(&val.to_string());
                                    arg_idx += 1;
                                }
                            }
                            Some('f') => {
                                chars.next();
                                if arg_idx < arg_values.len() {
                                    let val = arg_values[arg_idx].to_float().unwrap_or(0.0);
                                    result.push_str(&val.to_string());
                                    arg_idx += 1;
                                }
                            }
                            Some('c') => {
                                chars.next();
                                if arg_idx < arg_values.len() {
                                    let code = arg_values[arg_idx].to_int().unwrap_or(0) as u32;
                                    if let Some(ch) = char::from_u32(code) {
                                        result.push(ch);
                                    }
                                    arg_idx += 1;
                                }
                            }
                            Some('-') | Some('0'..='9') => {
                                // Parse width/precision specifiers
                                let mut spec = String::new();
                                while let Some(&ch) = chars.peek() {
                                    if ch == '-' || ch == '.' || ch.is_ascii_digit() {
                                        spec.push(ch);
                                        chars.next();
                                    } else {
                                        break;
                                    }
                                }
                                // Get the format character
                                if let Some(fc) = chars.next() {
                                    if arg_idx < arg_values.len() {
                                        let formatted = match fc {
                                            's' => {
                                                let s = arg_values[arg_idx].to_string_value();
                                                arg_idx += 1;
                                                format_with_width(&s, &spec, false)
                                            }
                                            'd' | 'i' => {
                                                let val = arg_values[arg_idx].to_int().unwrap_or(0);
                                                arg_idx += 1;
                                                format_with_width(&val.to_string(), &spec, true)
                                            }
                                            'f' => {
                                                let val = arg_values[arg_idx].to_float().unwrap_or(0.0);
                                                arg_idx += 1;
                                                format_float_with_precision(val, &spec)
                                            }
                                            _ => format!("%{}{}", spec, fc),
                                        };
                                        result.push_str(&formatted);
                                    }
                                }
                            }
                            _ => {
                                result.push('%');
                            }
                        }
                    } else {
                        result.push(c);
                    }
                }

                Ok(TfValue::String(result))
            }

            // getopts(optstring, arglist) - parse command-line options
            "getopts" => {
                if args.len() != 2 {
                    return Err("getopts requires 2 arguments (optstring, arglist)".to_string());
                }
                let optstring = self.eval(&args[0])?.to_string_value();
                let arglist = self.eval(&args[1])?.to_string_value();

                // Parse optstring to find which options take values (followed by :)
                let mut opts_with_values = std::collections::HashSet::new();
                let mut chars = optstring.chars().peekable();
                while let Some(c) = chars.next() {
                    if chars.peek() == Some(&':') {
                        opts_with_values.insert(c);
                        chars.next();
                    }
                }

                // Parse arguments
                let args_vec: Vec<&str> = arglist.split_whitespace().collect();
                let mut i = 0;
                while i < args_vec.len() {
                    let arg = args_vec[i];
                    if arg.starts_with('-') && arg.len() > 1 {
                        let opt_char = arg.chars().nth(1).unwrap();
                        let var_name = format!("opt_{}", opt_char);

                        if opts_with_values.contains(&opt_char) {
                            // Option takes a value
                            let value = if arg.len() > 2 {
                                // Value attached: -fvalue
                                arg[2..].to_string()
                            } else if i + 1 < args_vec.len() {
                                // Value is next argument
                                i += 1;
                                args_vec[i].to_string()
                            } else {
                                String::new()
                            };
                            self.engine.set_global(&var_name, TfValue::String(value));
                        } else {
                            // Boolean flag
                            self.engine.set_global(&var_name, TfValue::Integer(1));
                        }
                    }
                    i += 1;
                }

                Ok(TfValue::Integer(1))
            }

            // fg_world() - get foreground (current) world name
            "fg_world" => {
                let world_name = self.engine.current_world.clone().unwrap_or_default();
                Ok(TfValue::String(world_name))
            }

            // world_info(world, field) - get information about a world
            "world_info" => {
                if args.len() != 2 {
                    return Err("world_info requires 2 arguments (world, field)".to_string());
                }
                let world_name = self.eval(&args[0])?.to_string_value();
                let field = self.eval(&args[1])?.to_string_value();

                // Find world in cache
                let world = self.engine.world_info_cache.iter()
                    .find(|w| w.name.eq_ignore_ascii_case(&world_name));

                match world {
                    Some(w) => {
                        let value = match field.to_lowercase().as_str() {
                            "name" => TfValue::String(w.name.clone()),
                            "host" => TfValue::String(w.host.clone()),
                            "port" => TfValue::String(w.port.clone()),
                            "character" | "char" => TfValue::String(w.user.clone()),
                            "login" => TfValue::Integer(if w.is_connected { 1 } else { 0 }),
                            "ssl" | "secure" => TfValue::Integer(if w.use_ssl { 1 } else { 0 }),
                            _ => TfValue::String(String::new()),
                        };
                        Ok(value)
                    }
                    None => Ok(TfValue::String(String::new())),
                }
            }

            // ismacro(name) - check if a macro exists
            "ismacro" => {
                if args.len() != 1 {
                    return Err("ismacro requires 1 argument (name)".to_string());
                }
                let name = self.eval(&args[0])?.to_string_value();
                let exists = self.engine.macros.iter().any(|m| m.name == name);
                Ok(TfValue::Integer(if exists { 1 } else { 0 }))
            }

            // nactive() - count active (connected) worlds
            "nactive" => {
                let count = self.engine.world_info_cache.iter()
                    .filter(|w| w.is_connected)
                    .count();
                Ok(TfValue::Integer(count as i64))
            }

            // nworlds() - count total worlds
            "nworlds" => {
                let count = self.engine.world_info_cache.len();
                Ok(TfValue::Integer(count as i64))
            }

            // nread(world) - bytes available to read (always 0 - we don't buffer socket reads)
            "nread" => {
                Ok(TfValue::Integer(0))
            }

            // nlog() - lines in current log buffer (always 0 - we write directly)
            "nlog" => {
                Ok(TfValue::Integer(0))
            }

            // is_connected(world) - check if world is connected
            "is_connected" => {
                if args.is_empty() {
                    // Check current world
                    let current = self.engine.current_world.clone().unwrap_or_default();
                    let connected = self.engine.world_info_cache.iter()
                        .find(|w| w.name == current)
                        .map(|w| w.is_connected)
                        .unwrap_or(false);
                    Ok(TfValue::Integer(if connected { 1 } else { 0 }))
                } else {
                    let world_name = self.eval(&args[0])?.to_string_value();
                    let connected = self.engine.world_info_cache.iter()
                        .find(|w| w.name.eq_ignore_ascii_case(&world_name))
                        .map(|w| w.is_connected)
                        .unwrap_or(false);
                    Ok(TfValue::Integer(if connected { 1 } else { 0 }))
                }
            }

            // idle() - seconds since last user input (not tracked, return 0)
            "idle" => {
                Ok(TfValue::Integer(0))
            }

            // sidle() - seconds since last send to server (not tracked, return 0)
            "sidle" => {
                Ok(TfValue::Integer(0))
            }

            // columns() - number of columns on screen
            "columns" => {
                // Return a reasonable default; actual terminal width not tracked in TfEngine
                Ok(TfValue::Integer(80))
            }

            // lines() - number of lines on screen
            "lines" => {
                // Return a reasonable default; actual terminal height not tracked in TfEngine
                Ok(TfValue::Integer(24))
            }

            // moresize() - lines queued at more prompt (always 0 - handled by main app)
            "moresize" => {
                Ok(TfValue::Integer(0))
            }

            // morescroll(n) - scroll n lines at more prompt (returns 1 if scrolled, 0 otherwise)
            "morescroll" => {
                // Not implemented - more mode is handled by main app
                Ok(TfValue::Integer(0))
            }

            // getpid() - process id
            "getpid" => {
                Ok(TfValue::Integer(std::process::id() as i64))
            }

            // systype() - system type
            "systype" => {
                #[cfg(target_os = "linux")]
                { Ok(TfValue::String("unix".to_string())) }
                #[cfg(target_os = "macos")]
                { Ok(TfValue::String("unix".to_string())) }
                #[cfg(target_os = "windows")]
                { Ok(TfValue::String("cygwin32".to_string())) }
                #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
                { Ok(TfValue::String("unix".to_string())) }
            }

            // nmail() - mail files with unread mail (always 0 - not implemented)
            "nmail" => {
                Ok(TfValue::Integer(0))
            }

            // filename(s) - perform filename expansion
            "filename" => {
                if args.len() != 1 {
                    return Err("filename requires 1 argument".to_string());
                }
                let path = self.eval(&args[0])?.to_string_value();
                // Expand ~ to home directory
                let expanded = if path.starts_with('~') {
                    if let Some(home) = std::env::var_os("HOME") {
                        let home_str = home.to_string_lossy();
                        if path == "~" {
                            home_str.to_string()
                        } else if let Some(rest) = path.strip_prefix("~/") {
                            format!("{}/{}", home_str, rest)
                        } else {
                            path
                        }
                    } else {
                        path
                    }
                } else {
                    path
                };
                Ok(TfValue::String(expanded))
            }

            // ftime(format, time) - format a time value
            "ftime" => {
                if args.len() != 2 {
                    return Err("ftime requires 2 arguments (format, time)".to_string());
                }
                let format = self.eval(&args[0])?.to_string_value();
                let timestamp = self.eval(&args[1])?.to_int().unwrap_or(0);

                // Basic strftime-like formatting
                use std::time::{Duration, UNIX_EPOCH};
                let datetime = UNIX_EPOCH + Duration::from_secs(timestamp as u64);
                let secs_since_epoch = datetime.duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                // Calculate time components (UTC)
                let secs = secs_since_epoch % 60;
                let mins = (secs_since_epoch / 60) % 60;
                let hours = (secs_since_epoch / 3600) % 24;
                let days_since_epoch = secs_since_epoch / 86400;

                // Simple date calculation (not accounting for leap years properly, but close enough)
                let mut year = 1970i64;
                let mut remaining_days = days_since_epoch as i64;
                loop {
                    let days_in_year = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 366 } else { 365 };
                    if remaining_days < days_in_year {
                        break;
                    }
                    remaining_days -= days_in_year;
                    year += 1;
                }

                let days_in_month = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
                let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
                let mut month = 1;
                for (i, &days) in days_in_month.iter().enumerate() {
                    let d = if i == 1 && leap { 29 } else { days };
                    if remaining_days < d {
                        break;
                    }
                    remaining_days -= d;
                    month += 1;
                }
                let day = remaining_days + 1;

                // Apply format substitutions
                let result = format
                    .replace("%Y", &format!("{:04}", year))
                    .replace("%m", &format!("{:02}", month))
                    .replace("%d", &format!("{:02}", day))
                    .replace("%H", &format!("{:02}", hours))
                    .replace("%M", &format!("{:02}", mins))
                    .replace("%S", &format!("{:02}", secs))
                    .replace("%%", "%");

                Ok(TfValue::String(result))
            }

            // fwrite(filename, text) - append text to file
            "fwrite" => {
                if args.len() != 2 {
                    return Err("fwrite requires 2 arguments (filename, text)".to_string());
                }
                let filename = self.eval(&args[0])?.to_string_value();
                let text = self.eval(&args[1])?.to_string_value();

                // Expand ~ in filename
                let expanded = if filename.starts_with("~/") {
                    if let Some(home) = std::env::var_os("HOME") {
                        format!("{}/{}", home.to_string_lossy(), &filename[2..])
                    } else {
                        filename
                    }
                } else {
                    filename
                };

                use std::io::Write;
                match std::fs::OpenOptions::new().create(true).append(true).open(&expanded) {
                    Ok(mut file) => {
                        match file.write_all(text.as_bytes()) {
                            Ok(_) => Ok(TfValue::Integer(1)),
                            Err(_) => Ok(TfValue::Integer(0)),
                        }
                    }
                    Err(_) => Ok(TfValue::Integer(0)),
                }
            }

            // kbhead() - text before cursor
            "kbhead" => {
                let kb = &self.engine.keyboard_state;
                let pos = kb.cursor_position.min(kb.buffer.len());
                Ok(TfValue::String(kb.buffer[..pos].to_string()))
            }

            // kbtail() - text after cursor
            "kbtail" => {
                let kb = &self.engine.keyboard_state;
                let pos = kb.cursor_position.min(kb.buffer.len());
                Ok(TfValue::String(kb.buffer[pos..].to_string()))
            }

            // kbpoint() - cursor position
            "kbpoint" => {
                Ok(TfValue::Integer(self.engine.keyboard_state.cursor_position as i64))
            }

            // kblen() - total input length
            "kblen" => {
                Ok(TfValue::Integer(self.engine.keyboard_state.buffer.len() as i64))
            }

            // kbgoto(pos) - move cursor to position
            "kbgoto" => {
                if args.len() != 1 {
                    return Err("kbgoto requires 1 argument (position)".to_string());
                }
                let pos = self.eval(&args[0])?.to_int().unwrap_or(0) as usize;
                self.engine.pending_keyboard_ops.push(super::PendingKeyboardOp::Goto(pos));
                Ok(TfValue::Integer(1))
            }

            // kbdel(count) - delete characters (positive = forward, negative = backward)
            "kbdel" => {
                if args.len() != 1 {
                    return Err("kbdel requires 1 argument (count)".to_string());
                }
                let count = self.eval(&args[0])?.to_int().unwrap_or(0) as i32;
                self.engine.pending_keyboard_ops.push(super::PendingKeyboardOp::Delete(count));
                Ok(TfValue::Integer(1))
            }

            // kbmatch() - find matching brace/paren (returns position or -1)
            "kbmatch" => {
                let kb = &self.engine.keyboard_state;
                let pos = kb.cursor_position.min(kb.buffer.len());
                if pos == 0 || pos > kb.buffer.len() {
                    return Ok(TfValue::Integer(-1));
                }

                let chars: Vec<char> = kb.buffer.chars().collect();
                let current_char = if pos > 0 && pos <= chars.len() {
                    chars[pos - 1]
                } else {
                    return Ok(TfValue::Integer(-1));
                };

                // Define matching pairs
                let (target, direction) = match current_char {
                    '(' => (')', 1i32),
                    ')' => ('(', -1),
                    '[' => (']', 1),
                    ']' => ('[', -1),
                    '{' => ('}', 1),
                    '}' => ('{', -1),
                    '<' => ('>', 1),
                    '>' => ('<', -1),
                    _ => return Ok(TfValue::Integer(-1)),
                };

                let mut depth = 1;
                let mut idx = (pos as i32 - 1) + direction;
                while idx >= 0 && (idx as usize) < chars.len() {
                    let c = chars[idx as usize];
                    if c == current_char {
                        depth += 1;
                    } else if c == target {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(TfValue::Integer(idx as i64 + 1)); // 1-indexed
                        }
                    }
                    idx += direction;
                }

                Ok(TfValue::Integer(-1))
            }

            // kbwordleft([pos]) - position of word start left of pos
            "kbwordleft" => {
                let kb = &self.engine.keyboard_state;
                let chars: Vec<char> = kb.buffer.chars().collect();
                let pos = if args.is_empty() {
                    kb.cursor_position
                } else {
                    self.eval(&args[0])?.to_int().unwrap_or(0) as usize
                };
                let pos = pos.min(chars.len());

                // Skip whitespace going left
                let mut i = pos;
                while i > 0 && chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                // Find start of word
                while i > 0 && !chars[i - 1].is_whitespace() {
                    i -= 1;
                }
                Ok(TfValue::Integer(i as i64))
            }

            // kbwordright([pos]) - position past word end right of pos
            "kbwordright" => {
                let kb = &self.engine.keyboard_state;
                let chars: Vec<char> = kb.buffer.chars().collect();
                let pos = if args.is_empty() {
                    kb.cursor_position
                } else {
                    self.eval(&args[0])?.to_int().unwrap_or(0) as usize
                };
                let pos = pos.min(chars.len());

                // Skip non-whitespace going right
                let mut i = pos;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                // Skip whitespace
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                Ok(TfValue::Integer(i as i64))
            }

            // kbword() - get word at cursor
            "kbword" => {
                let kb = &self.engine.keyboard_state;
                let chars: Vec<char> = kb.buffer.chars().collect();
                let pos = kb.cursor_position.min(chars.len());

                if pos == 0 || chars.is_empty() {
                    return Ok(TfValue::String(String::new()));
                }

                // Find word boundaries
                let mut start = pos;
                while start > 0 && chars[start - 1].is_alphanumeric() {
                    start -= 1;
                }

                let mut end = pos;
                while end < chars.len() && chars[end].is_alphanumeric() {
                    end += 1;
                }

                let word: String = chars[start..end].iter().collect();
                Ok(TfValue::String(word))
            }

            // input(text) - insert text at cursor
            "input" => {
                if args.is_empty() {
                    return Err("input requires at least 1 argument (text)".to_string());
                }
                let text = self.eval(&args[0])?.to_string_value();
                self.engine.pending_keyboard_ops.push(super::PendingKeyboardOp::Insert(text));
                Ok(TfValue::Integer(1))
            }

            // tfopen(filename, mode) - open a file (returns handle, 0 on failure)
            "tfopen" => {
                if args.len() < 2 {
                    return Err("tfopen requires 2 arguments (filename, mode)".to_string());
                }
                let filename = self.eval(&args[0])?.to_string_value();
                let mode_str = self.eval(&args[1])?.to_string_value();

                let mode = match mode_str.to_lowercase().as_str() {
                    "r" | "read" => super::TfFileMode::Read,
                    "w" | "write" => super::TfFileMode::Write,
                    "a" | "append" => super::TfFileMode::Append,
                    _ => return Err(format!("Invalid file mode: {} (use r, w, or a)", mode_str)),
                };

                // Verify the file can be opened in the specified mode
                let can_open = match mode {
                    super::TfFileMode::Read => std::path::Path::new(&filename).exists(),
                    super::TfFileMode::Write => {
                        // Try to create/truncate the file
                        std::fs::File::create(&filename).is_ok()
                    }
                    super::TfFileMode::Append => {
                        // Try to open for appending
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&filename)
                            .is_ok()
                    }
                };

                if !can_open {
                    return Ok(TfValue::Integer(0)); // Return 0 on failure (TF convention)
                }

                // Allocate a handle
                let handle = self.engine.next_file_handle;
                self.engine.next_file_handle += 1;

                self.engine.open_files.insert(handle, super::TfFileHandle {
                    path: filename,
                    mode,
                    read_position: 0,
                });

                Ok(TfValue::Integer(handle as i64))
            }

            // tfclose(handle) - close a file (returns 1 on success, 0 on failure)
            "tfclose" => {
                if args.len() != 1 {
                    return Err("tfclose requires 1 argument (handle)".to_string());
                }
                let handle = self.eval(&args[0])?.to_int().unwrap_or(-1) as i32;

                if self.engine.open_files.remove(&handle).is_some() {
                    Ok(TfValue::Integer(1))
                } else {
                    Ok(TfValue::Integer(0))
                }
            }

            // tfread(handle, varname) - read a line into variable (returns 1 on success, 0 on EOF/error)
            "tfread" => {
                if args.len() != 2 {
                    return Err("tfread requires 2 arguments (handle, varname)".to_string());
                }
                let handle = self.eval(&args[0])?.to_int().unwrap_or(-1) as i32;
                let varname = self.eval(&args[1])?.to_string_value();

                let file_handle = match self.engine.open_files.get_mut(&handle) {
                    Some(fh) if fh.mode == super::TfFileMode::Read => fh,
                    _ => return Ok(TfValue::Integer(0)),
                };

                // Open the file and seek to current position
                let file = match std::fs::File::open(&file_handle.path) {
                    Ok(f) => f,
                    Err(_) => return Ok(TfValue::Integer(0)),
                };

                use std::io::Seek;
                let mut reader = BufReader::new(file);
                if reader.seek(std::io::SeekFrom::Start(file_handle.read_position)).is_err() {
                    return Ok(TfValue::Integer(0));
                }

                // Read one line
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        // EOF
                        Ok(TfValue::Integer(0))
                    }
                    Ok(n) => {
                        // Update position
                        file_handle.read_position += n as u64;

                        // Strip trailing newline
                        if line.ends_with('\n') {
                            line.pop();
                            if line.ends_with('\r') {
                                line.pop();
                            }
                        }

                        // Set the variable
                        self.engine.set_global(&varname, TfValue::String(line));
                        Ok(TfValue::Integer(1))
                    }
                    Err(_) => Ok(TfValue::Integer(0)),
                }
            }

            // tfwrite(handle, text) - write text to file (returns 1 on success, 0 on failure)
            "tfwrite" => {
                if args.len() != 2 {
                    return Err("tfwrite requires 2 arguments (handle, text)".to_string());
                }
                let handle = self.eval(&args[0])?.to_int().unwrap_or(-1) as i32;
                let text = self.eval(&args[1])?.to_string_value();

                let file_handle = match self.engine.open_files.get(&handle) {
                    Some(fh) if fh.mode == super::TfFileMode::Write || fh.mode == super::TfFileMode::Append => fh,
                    _ => return Ok(TfValue::Integer(0)),
                };

                // Open file in appropriate mode
                let mut file = match file_handle.mode {
                    super::TfFileMode::Write => {
                        match std::fs::OpenOptions::new()
                            
                            .create(true)
                            .append(true) // Use append to not overwrite on each write
                            .open(&file_handle.path)
                        {
                            Ok(f) => f,
                            Err(_) => return Ok(TfValue::Integer(0)),
                        }
                    }
                    super::TfFileMode::Append => {
                        match std::fs::OpenOptions::new()
                            .append(true)
                            .create(true)
                            .open(&file_handle.path)
                        {
                            Ok(f) => f,
                            Err(_) => return Ok(TfValue::Integer(0)),
                        }
                    }
                    _ => return Ok(TfValue::Integer(0)),
                };

                // Write with newline
                match writeln!(file, "{}", text) {
                    Ok(_) => Ok(TfValue::Integer(1)),
                    Err(_) => Ok(TfValue::Integer(0)),
                }
            }

            // tfflush(handle) - flush file (returns 1 on success, 0 on failure)
            "tfflush" => {
                if args.len() != 1 {
                    return Err("tfflush requires 1 argument (handle)".to_string());
                }
                let handle = self.eval(&args[0])?.to_int().unwrap_or(-1) as i32;

                // Check if handle is valid
                if self.engine.open_files.contains_key(&handle) {
                    // File operations are not buffered in our implementation, so flush is a no-op
                    Ok(TfValue::Integer(1))
                } else {
                    Ok(TfValue::Integer(0))
                }
            }

            // tfeof(handle) - check if at end of file (returns 1 if at EOF, 0 otherwise)
            "tfeof" => {
                if args.len() != 1 {
                    return Err("tfeof requires 1 argument (handle)".to_string());
                }
                let handle = self.eval(&args[0])?.to_int().unwrap_or(-1) as i32;

                let file_handle = match self.engine.open_files.get(&handle) {
                    Some(fh) if fh.mode == super::TfFileMode::Read => fh,
                    _ => return Ok(TfValue::Integer(1)), // Invalid handle = EOF
                };

                // Check if we're at EOF by comparing position to file size
                match std::fs::metadata(&file_handle.path) {
                    Ok(meta) => {
                        let at_eof = file_handle.read_position >= meta.len();
                        Ok(TfValue::Integer(if at_eof { 1 } else { 0 }))
                    }
                    Err(_) => Ok(TfValue::Integer(1)),
                }
            }

            // echo(s [,attrs [,dest [,inline]]]) - function form of #echo
            // Returns 1 on success
            "echo" => {
                if args.is_empty() {
                    return Err("echo requires at least 1 argument (text)".to_string());
                }
                let text = self.eval(&args[0])?.to_string_value();
                let attrs = if args.len() > 1 {
                    self.eval(&args[1])?.to_string_value()
                } else {
                    String::new()
                };
                // dest and inline are ignored in our implementation
                // Queue the echo for main app to process
                self.engine.pending_outputs.push(super::TfOutput {
                    text,
                    attrs,
                    world: None,
                });
                Ok(TfValue::Integer(1))
            }

            // send(s [,world [,flags]]) - function form of #send
            // Returns 1 on success
            "send" => {
                if args.is_empty() {
                    return Err("send requires at least 1 argument (text)".to_string());
                }
                let text = self.eval(&args[0])?.to_string_value();
                let world = if args.len() > 1 {
                    let w = self.eval(&args[1])?.to_string_value();
                    if w.is_empty() { None } else { Some(w) }
                } else {
                    None
                };
                // flags argument is ignored in our implementation
                self.engine.pending_commands.push(super::TfCommand {
                    command: text,
                    world,
                    no_eol: false,
                });
                Ok(TfValue::Integer(1))
            }

            // substitute(s [,attrs [,inline]]) - replace trigger text with substituted text
            // Returns 1 on success (but only works during trigger execution)
            "substitute" => {
                if args.is_empty() {
                    return Err("substitute requires at least 1 argument (text)".to_string());
                }
                let text = self.eval(&args[0])?.to_string_value();
                let attrs = if args.len() > 1 {
                    self.eval(&args[1])?.to_string_value()
                } else {
                    String::new()
                };
                // inline is ignored - we always substitute inline
                // Queue the substitution for main app to process
                self.engine.pending_substitution = Some(super::TfSubstitution {
                    text,
                    attrs,
                });
                Ok(TfValue::Integer(1))
            }

            // keycode(s) - return the key sequence that generates the given string
            // This is the inverse of key binding - what keys produce this character
            "keycode" => {
                if args.len() != 1 {
                    return Err("keycode requires 1 argument (string)".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();

                // Return the key sequence representation
                // For control characters, return ^X format
                // For regular characters, return as-is
                let mut result = String::new();
                for c in s.chars() {
                    let code = c as u32;
                    if code < 32 {
                        // Control character
                        result.push('^');
                        result.push(char::from_u32(code + 64).unwrap_or('?'));
                    } else if code == 127 {
                        // DEL
                        result.push_str("^?");
                    } else if code >= 128 {
                        // Meta/Alt character - represent as @X
                        result.push('@');
                        result.push(char::from_u32(code - 128).unwrap_or('?'));
                    } else {
                        result.push(c);
                    }
                }
                Ok(TfValue::String(result))
            }

            _ => Err(format!("Unknown function: {}", name)),
        }
    }
}

/// Format a string with width specification (for sprintf)
fn format_with_width(s: &str, spec: &str, numeric: bool) -> String {
    let mut left_align = false;
    let mut zero_pad = false;
    let mut width = 0;
    let mut spec_chars = spec.chars().peekable();

    // Parse flags
    while let Some(&c) = spec_chars.peek() {
        match c {
            '-' => { left_align = true; spec_chars.next(); }
            '0' if width == 0 && numeric => { zero_pad = true; spec_chars.next(); }
            _ => break,
        }
    }

    // Parse width
    let width_str: String = spec_chars.take_while(|c| c.is_ascii_digit()).collect();
    if !width_str.is_empty() {
        width = width_str.parse().unwrap_or(0);
    }

    if width == 0 || s.len() >= width {
        return s.to_string();
    }

    let padding = width - s.len();
    let pad_char = if zero_pad && !left_align { '0' } else { ' ' };

    if left_align {
        format!("{}{}", s, pad_char.to_string().repeat(padding))
    } else {
        format!("{}{}", pad_char.to_string().repeat(padding), s)
    }
}

/// Format a float with precision specification (for sprintf)
fn format_float_with_precision(val: f64, spec: &str) -> String {
    // Parse width and precision from spec like "10.2"
    let parts: Vec<&str> = spec.split('.').collect();
    let precision = if parts.len() > 1 {
        parts[1].parse().unwrap_or(6)
    } else {
        6
    };

    let formatted = format!("{:.prec$}", val, prec = precision);

    if !parts.is_empty() && !parts[0].is_empty() {
        let width: usize = parts[0].trim_start_matches('-').trim_start_matches('0').parse().unwrap_or(0);
        let left_align = parts[0].starts_with('-');
        if width > formatted.len() {
            let padding = width - formatted.len();
            if left_align {
                format!("{}{}", formatted, " ".repeat(padding))
            } else {
                format!("{}{}", " ".repeat(padding), formatted)
            }
        } else {
            formatted
        }
    } else {
        formatted
    }
}

/// Convert a glob pattern to a regex pattern
/// Supports \* and \? to match literal asterisk and question mark
fn glob_to_regex(pattern: &str) -> String {
    let mut result = String::from("^");

    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // Check for escape sequences
                match chars.peek() {
                    Some('*') | Some('?') | Some('\\') => {
                        // Escaped wildcard or backslash - treat as literal
                        let escaped = chars.next().unwrap();
                        result.push('\\');
                        result.push(escaped);
                    }
                    _ => {
                        // Lone backslash - escape it for regex
                        result.push_str("\\\\");
                    }
                }
            }
            '*' => result.push_str(".*"),
            '?' => result.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' => {
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

    #[test]
    fn test_echo_function() {
        let mut engine = TfEngine::new();
        // echo() should queue an output and return 1
        let result = evaluate(&mut engine, r#"echo("Hello world")"#).unwrap();
        assert_eq!(result, TfValue::Integer(1));
        assert_eq!(engine.pending_outputs.len(), 1);
        assert_eq!(engine.pending_outputs[0].text, "Hello world");
    }

    #[test]
    fn test_send_function() {
        let mut engine = TfEngine::new();
        // send() should queue a command and return 1
        let result = evaluate(&mut engine, r#"send("look")"#).unwrap();
        assert_eq!(result, TfValue::Integer(1));
        assert_eq!(engine.pending_commands.len(), 1);
        assert_eq!(engine.pending_commands[0].command, "look");
    }

    #[test]
    fn test_substitute_function() {
        let mut engine = TfEngine::new();
        // substitute() should set pending_substitution and return 1
        let result = evaluate(&mut engine, r#"substitute("replaced text")"#).unwrap();
        assert_eq!(result, TfValue::Integer(1));
        assert!(engine.pending_substitution.is_some());
        assert_eq!(engine.pending_substitution.unwrap().text, "replaced text");
    }

    #[test]
    fn test_keycode_function() {
        let mut engine = TfEngine::new();
        // Regular characters return as-is
        assert_eq!(evaluate(&mut engine, r#"keycode("abc")"#).unwrap(), TfValue::String("abc".to_string()));
        // Control characters return ^X format
        assert_eq!(evaluate(&mut engine, "keycode(char(1))").unwrap(), TfValue::String("^A".to_string()));
        assert_eq!(evaluate(&mut engine, "keycode(char(3))").unwrap(), TfValue::String("^C".to_string()));
        // DEL returns ^?
        assert_eq!(evaluate(&mut engine, "keycode(char(127))").unwrap(), TfValue::String("^?".to_string()));
    }
}
