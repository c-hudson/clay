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

    // Command/expression/macro substitution operands - real TF's own
    // "expressions" help lists these as legitimate Operand forms alongside
    // string/numeric literals and {selector} ("Command substitutions like
    // $(/listworlds -s)", "Macro substitutions like ${COMPRESS_SUFFIX}",
    // and $[...] itself - accepted with a "legal, but redundant" warning -
    // verified directly against real tf 5.0 beta 8). Each token carries the
    // still-unsubstituted, balance-extracted text between its delimiters;
    // it is resolved lazily by the evaluator (Expr::CommandSub/ExprSub/
    // MacroSub), not eagerly during lexing, so a ternary/&&/|| branch that
    // is never taken never runs its command substitution's side effects
    // (lisp.tf's own `/unique` recursion depends on exactly this - see the
    // job report for the "$(...) inside $[...]" investigation this fixes).
    CommandSub(String), // $(...)
    ExprSub(String),    // $[...]
    MacroSub(String),   // ${...}

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

        // $(...) / $[...] / ${...} - command/expression/macro substitution
        // operands (see the Token variants' own doc comment). Extraction
        // only tracks the matching delimiter pair (mirroring
        // variables::extract_balanced, reused here), so e.g. the "{stack}"
        // inside "$(/cdr %{stack})" is just plain content to the paren
        // count, not a nesting hazard.
        if c == '$' {
            if let Some(next) = self.peek_next() {
                if next == '(' || next == '[' || next == '{' {
                    self.advance(); // consume '$'
                    self.advance(); // consume the opening delimiter
                    let (close, make_tok): (char, fn(String) -> Token) = match next {
                        '(' => (')', Token::CommandSub),
                        '[' => (']', Token::ExprSub),
                        _ => ('}', Token::MacroSub),
                    };
                    return match super::variables::extract_balanced(&self.chars, self.pos, next, close) {
                        Some((content, end_idx)) => {
                            self.pos = end_idx + 1;
                            Ok(make_tok(content))
                        }
                        None => Err(format!("Unterminated ${}...", next)),
                    };
                }
            }
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
            '#' => Ok(Token::Identifier("#".to_string())),
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

    /// Parse a full expression, including TF's lowest-precedence comma
    /// operator. Only the genuine top-level entry point (this function, and
    /// the "(e)" grouping case in parse_primary) should call this - a
    /// function call's own argument list uses `,` as the argument
    /// separator, not the sequencing operator, so parse_args calls
    /// parse_assignment directly instead (see its own doc comment).
    pub fn parse(&mut self) -> Result<Expr, String> {
        self.parse_comma()
    }

    // Precedence levels (lowest to highest):
    // 1. Comma (,)
    // 2. Assignment (:=)
    // 3. Ternary (?:)
    // 4. Logical OR (|)
    // 5. Logical AND (&)
    // 6. Equality (==, !=, =~, !~, =/, !/)
    // 7. Comparison (<, <=, >, >=)
    // 8. Addition (+, -)
    // 9. Multiplication (*, /, %)
    // 10. Unary (!, -, ++, --)
    // 11. Primary (literals, variables, function calls, parentheses)

    fn parse_comma(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_assignment()?;

        while self.peek() == &Token::Comma {
            self.advance();
            let right = self.parse_assignment()?;
            expr = Expr::BinaryOp(Box::new(expr), BinaryOp::Comma, Box::new(right));
        }

        Ok(expr)
    }

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
            //
            // The true branch is parsed at COMMA precedence (not `or`),
            // matching C's own ternary grammar (and verified directly
            // against real tf: "1 ? 2,3 : 4" evaluates to 3 with no parens
            // needed) - the comma operator is only excluded from the FALSE
            // branch and the condition itself. stdlib.tf's own `/nth`
            // depends on exactly this: `{1} > 0 ? shift({1}), {1} : ""`
            // needs "shift({1}), {1}" to parse as one comma-expression
            // (evaluate the shift for its side effect, then read the -now
            // shifted- {1}), not stop at the first comma and choke on a
            // stray ":" it can never find.
            let true_expr = if self.peek() == &Token::Colon {
                Box::new(condition.clone())
            } else {
                Box::new(self.parse_comma()?)
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
                // {varname}, {*}, {n}, {-n}, {L}/{LN} (Nth positional
                // parameter from the end), {-L}/{-LN} (all but the last N) -
                // see /help substitution's selector grammar, and
                // variables::resolve_extended_selector for the shared
                // "-N"/"L"/"-L" evaluation (also used by the "%..." text-
                // substitution forms in variables.rs). A trailing
                // "-default" after a plain selector (e.g. "{1-DEF}") is
                // accepted but the default itself is only implemented for
                // the "%{...}" text-substitution form; here it just falls
                // back to the selector's own value (empty if unset), which
                // matches real TF whenever the selector isn't actually
                // empty - nothing in this codebase's test corpus needs a
                // real default evaluated inside a bare "{...}" expression.
                self.advance();

                let is_star = matches!(self.peek(), Token::Star);
                let is_minus = matches!(self.peek(), Token::Minus);
                let integer_val = if let Token::Integer(n) = self.peek() { Some(*n) } else { None };
                let ident_val = if let Token::Identifier(s) = self.peek() { Some(s.clone()) } else { None };

                // Consume an optional "-default" suffix after a selector
                // has already been parsed (just before the closing brace).
                let skip_default_suffix = |p: &mut Self| -> Result<(), String> {
                    if matches!(p.peek(), Token::Minus) {
                        p.advance();
                        if !matches!(p.peek(), Token::RBrace) {
                            p.advance();
                        }
                    }
                    Ok(())
                };

                if is_star {
                    // {*} - all arguments
                    self.advance();
                    self.expect(Token::RBrace)?;
                    Ok(Expr::Variable("*".to_string()))
                } else if is_minus {
                    // {-n} / {-Ln} - "except first n" / "except last n"
                    self.advance();
                    match self.peek().clone() {
                        Token::Integer(n) => {
                            self.advance();
                            self.expect(Token::RBrace)?;
                            Ok(Expr::Variable(format!("-{}", n)))
                        }
                        Token::Identifier(s) if s.strip_prefix('L').is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit())) => {
                            self.advance();
                            self.expect(Token::RBrace)?;
                            Ok(Expr::Variable(format!("-{}", s)))
                        }
                        other => Err(format!("Expected number or L<n> after - in {{-n}}, got {:?}", other)),
                    }
                } else if let Some(n) = integer_val {
                    // {n} or {n-default}
                    self.advance();
                    skip_default_suffix(self)?;
                    self.expect(Token::RBrace)?;
                    Ok(Expr::Variable(n.to_string()))
                } else if let Some(name) = ident_val {
                    // {varname}, {L}/{Ln}, or either with a "-default" suffix
                    self.advance();
                    skip_default_suffix(self)?;
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
            Token::CommandSub(cmd) => {
                self.advance();
                Ok(Expr::CommandSub(cmd))
            }
            Token::ExprSub(expr_text) => {
                self.advance();
                Ok(Expr::ExprSub(expr_text))
            }
            Token::MacroSub(name) => {
                self.advance();
                Ok(Expr::MacroSub(name))
            }
            _ => Err(format!("Unexpected token: {:?}", self.peek())),
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();

        if self.peek() == &Token::RParen {
            return Ok(args);
        }

        // Each argument is parsed at assignment precedence, NOT via
        // parse()/parse_comma() - here, "," is the argument separator, not
        // TF's comma-sequencing operator (that operator is only reachable
        // when explicitly parenthesized, e.g. "f((a,b), c)" - same rule as
        // C, which TF's expression grammar is modeled on).
        args.push(self.parse_assignment()?);

        while self.peek() == &Token::Comma {
            self.advance();
            args.push(self.parse_assignment()?);
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
    // Sequencing: `e1, e2` evaluates both, left to right, and yields e2's
    // value (see /help expressions' operator table - TF's lowest-precedence
    // operator, "only useful if e1 has some side effect"). Used throughout
    // real TF library scripts as `/while (shift(), {#}) ...` or
    // `/test (result:=result*n), --n` - two side effects in one call.
    Comma,
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
    /// $(...) command substitution as an expression operand - see the
    /// Token::CommandSub doc comment. Resolved lazily by the evaluator.
    CommandSub(String),
    /// $[...] nested expression substitution as an operand (Token::ExprSub).
    ExprSub(String),
    /// ${...} macro/variable substitution as an operand (Token::MacroSub).
    MacroSub(String),
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
                // "-N" (all positional parameters except the first N), "L"/
                // "LN" (Nth from the end) and "-L"/"-LN" (all but the last
                // N) are the selector forms that aren't just a plain local-
                // variable lookup - see resolve_extended_selector's doc
                // comment (shared with the "%..." text-substitution forms
                // in variables.rs, so both contexts agree). Verified
                // against real TinyFugue directly: for args "a b c d",
                // "{-1}" is "b c d" (except the first), not "d" - this used
                // to compute the latter (TF's "L1" meaning) here instead.
                if let Some(value) = super::variables::resolve_extended_selector(self.engine, name) {
                    return Ok(TfValue::String(value));
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
                // TF's `:=` (finding 20): update the binding wherever it
                // already lives, else create it as a global - never just
                // the innermost local scope. See
                // `TfEngine::set_existing_or_global`'s doc comment.
                let val = self.eval(value)?;
                self.engine.set_existing_or_global(name, val.clone());
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
                // ++/-- follow the same "update wherever it lives" rule as
                // `:=` (finding 20).
                self.engine.set_existing_or_global(name, new_val.clone());
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
                self.engine.set_existing_or_global(name, new_val.clone());
                Ok(new_val)
            }

            Expr::FunctionCall(name, args) => {
                self.eval_function(name, args)
            }

            // $(...) as an expression operand (see the Token::CommandSub
            // doc comment): mirror variables::substitute_commands' own
            // top-level $() handling exactly - expand %vars in the
            // extracted command text (any further-nested $(...)/$[...] in
            // there is resolved when the invoked command's own arguments
            // are substituted during its normal execution, not here), run
            // it, and use its output as this operand's string value. A
            // command error becomes an inline "[error: ...]" string rather
            // than aborting the whole expression, matching
            // execute_for_substitution's existing convention for the
            // text-substitution case.
            Expr::CommandSub(cmd_text) => {
                // Full substitution (not just %vars), matching
                // variables::substitute_commands' own top-level $(...)
                // handler and for the same reason - a $(...) inside an
                // expression can itself contain another $(...)/$[...]/
                // ${...} (lisp.tf's own `/unique` recursion).
                let cmd = super::variables::substitute_commands(self.engine, cmd_text);
                let output = super::variables::execute_for_substitution(self.engine, &cmd);
                Ok(TfValue::String(output))
            }

            // $[...] nested inside another expression - real tf accepts
            // this (with a "legal, but redundant" warning; verified
            // directly against real tf 5.0 beta 8) and evaluates it as a
            // genuine sub-expression whose VALUE becomes the operand - not
            // textual splicing, which would turn a string result into a
            // bareword variable-reference lookup instead of a literal.
            Expr::ExprSub(expr_text) => {
                let sub = super::variables::substitute_dollar_braces(self.engine, expr_text);
                evaluate(self.engine, &sub)
            }

            // ${...} - macro/variable substitution as an operand, matching
            // variables::substitute_commands' own top-level "${varname}"
            // handling: a variable's value, else a same-named simple
            // macro's (no trigger, no hook) body text, else empty.
            Expr::MacroSub(name) => {
                if let Some(value) = self.engine.get_var(name) {
                    Ok(value.clone())
                } else if let Some(macro_def) = self.engine.macros.iter()
                    .find(|m| m.name == *name && m.trigger.is_none() && m.hook.is_none())
                {
                    Ok(TfValue::String(macro_def.body.clone()))
                } else {
                    Ok(TfValue::String(String::new()))
                }
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
            // Arithmetic. Integer ops use wrapping arithmetic, not `+`/`-`/`*`
            // directly: a script whose recursion doesn't bottom out the way
            // Clay expects (e.g. a macro using /result to return a value -
            // finding C.5 - before that's implemented) can otherwise build an
            // i64 product large enough to overflow, which panics the whole
            // process in a debug build (and silently UB-free-but-still-wrong
            // wraps in release) - surfaced by lib_factoral.tf once P1.3 let
            // /require actually reach factoral.tf's rfact()/ifact(). A script
            // should never be able to crash the client; wrapping matches what
            // most native TF builds' underlying C `long` arithmetic does too.
            BinaryOp::Add => self.eval_arithmetic(&left_val, &right_val, |a, b| a.wrapping_add(b), |a, b| a + b),
            BinaryOp::Sub => self.eval_arithmetic(&left_val, &right_val, |a, b| a.wrapping_sub(b), |a, b| a - b),
            BinaryOp::Mul => self.eval_arithmetic(&left_val, &right_val, |a, b| a.wrapping_mul(b), |a, b| a * b),
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

            // e1, e2 - both already evaluated above, left to right; the
            // comma operator's value is simply e2's.
            BinaryOp::Comma => Ok(right_val),

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
                // Real TF's strings carry attributes out-of-band, so
                // strlen() never counts an attribute byte, only visible
                // text - verified against tf-lib's cylon.tf, whose
                // strlen(cylon0) (after decode_attr()) is exactly the
                // number of *visible* characters. Clay represents an
                // attributed string pragmatically as plain text with
                // embedded ANSI/`@{...}` codes (see decode_attr's doc
                // comment), so strlen() has to strip those back out before
                // counting - see `parser::strip_all_attributes`. This is a
                // no-op for any ordinary string, since one never contains
                // that markup in the first place.
                let visible = super::parser::strip_all_attributes(&s);
                Ok(TfValue::Integer(visible.chars().count() as i64))
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
                    let char_len = s.chars().count();
                    if char_len >= abs_width {
                        result.push_str(&s);
                    } else {
                        let padding = " ".repeat(abs_width - char_len);
                        if width >= 0 {
                            // Right-justify (left-pad)
                            result.push_str(&padding);
                            result.push_str(&s);
                        } else {
                            // Left-justify (right-pad)
                            result.push_str(&s);
                            result.push_str(&padding);
                        }
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

            "escape" => {
                if args.len() != 2 {
                    return Err("escape requires 2 arguments: escape(metacharacters, string)".to_string());
                }
                let metacharacters = self.eval(&args[0])?.to_string_value();
                let string = self.eval(&args[1])?.to_string_value();
                Ok(TfValue::String(super::parser::tf_escape(&metacharacters, &string)))
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
                    let range = (max as u64).wrapping_sub(min as u64).wrapping_add(1);
                    let r = min.wrapping_add(((simple_random() as u64) % range) as i64);
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

                // Get or compile regex (cached)
                if !self.regex_cache.contains_key(&pattern) {
                    match Regex::new(&pattern) {
                        Ok(r) => { self.regex_cache.insert(pattern.clone(), r); }
                        Err(e) => return Err(format!("Invalid regex: {}", e)),
                    }
                }
                let regex = self.regex_cache.get(&pattern).unwrap();

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
                    // Also expose P0-P9/PL/PR as ordinary local variables,
                    // matching what a trigger match does
                    // (macros::execute_macro_with_context) - real tf's own
                    // "/help substitution" says the %P subs "get their
                    // values from the last successful regexp match in
                    // scope ... or in which it occurred (i.e., with
                    // regmatch())", and %PL/%PR already read from local
                    // vars (variables::substitute_variables), not from
                    // regex_captures. Without this, textencode.tf's own
                    // "{PL}"/"{P0}"/"{PR}" (the bare EXPRESSION-brace form,
                    // evaluated here via Expr::Variable, which only ever
                    // checks local vars) stayed stuck on whatever a prior
                    // trigger left them at - verified directly against real
                    // tf that a bare regmatch() call updates them the same
                    // way a trigger match does.
                    let captured: Vec<String> = self.engine.regex_captures.iter().take(10).cloned().collect();
                    for (i, cap) in captured.into_iter().enumerate() {
                        self.engine.set_local(&format!("P{}", i), TfValue::String(cap));
                    }
                    let whole = caps.get(0).unwrap();
                    self.engine.set_local("PL", TfValue::String(text[..whole.start()].to_string()));
                    self.engine.set_local("PR", TfValue::String(text[whole.end()..].to_string()));
                    Ok(TfValue::Integer(1))
                } else {
                    // No match - clear captures (and the same P0-P9/PL/PR
                    // locals, matching real tf's own observed behavior:
                    // a failed regmatch() clears them rather than leaving
                    // a stale prior match in place).
                    for _ in 0..10 {
                        self.engine.regex_captures.push(String::new());
                    }
                    for i in 0..10 {
                        self.engine.set_local(&format!("P{}", i), TfValue::String(String::new()));
                    }
                    self.engine.set_local("PL", TfValue::String(String::new()));
                    self.engine.set_local("PR", TfValue::String(String::new()));
                    Ok(TfValue::Integer(0))
                }
            }

            // replace(old, new, str [, count]) - string replacement.
            // TF argument order (finding B's replace() ruling / plan step
            // P1.10): real tf's replace(s1, s2, s3) returns s3 with every
            // occurrence of s1 replaced by s2 - Clay used to take
            // (str, old, new) instead, which silently produced the wrong
            // result for anyone porting a real TF script. Clay's own
            // /replace command already used TF's order, so this is a
            // release-note-worthy behavior change to the *function* only.
            // The optional 4th `count` argument (limit how many occurrences
            // are replaced) is a Clay-only extension beyond TF's exact
            // 3-argument replace(s1, s2, s3).
            "replace" => {
                if args.len() < 3 || args.len() > 4 {
                    return Err("replace requires 3 or 4 arguments (old, new, str [, count])".to_string());
                }
                let old = self.eval(&args[0])?.to_string_value();
                let new = self.eval(&args[1])?.to_string_value();
                let text = self.eval(&args[2])?.to_string_value();

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

                let pos = text.char_indices().zip(0..).find_map(|((byte_pos, _), char_pos)| {
                    if text[byte_pos..].starts_with(&*substr) { Some(char_pos as i64) } else { None }
                }).unwrap_or(-1);
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
            // getopts(optstring [, init]) - parse the CURRENT macro's own
            // positional parameters (%1.. / %*), NOT a separate argument
            // list (finding C.11 / plan step P1.10's getopts() fix - the
            // old implementation took a literal arg-list string as its
            // 2nd argument, which isn't what real tf's 2-argument form
            // means at all: it's the *initial value* every opt_x local is
            // set to before parsing, used so a caller can tell "flag was
            // off" apart from "flag wasn't even mentioned"). <optstring>
            // letters may carry a ":" (string argument), "#" (integer
            // expression argument) or "@" (time argument, treated as a
            // plain string here - Clay has no separate time-argument type)
            // suffix; a bare letter is a boolean flag. Per `/help options`,
            // an option's argument must be attached to the same token (no
            // space) - possibly via the bundled-cluster form (e.g. "-n5" or
            // "-abn5"), same as `/def`'s own bundled short options (finding
            // 24, `macros::parse_option_char`). On success, every letter in
            // <optstring> found on the command line gets a local `opt_X`
            // variable, and the consumed leading tokens are shifted out of
            // %1../%*/%# - exactly like `/shift` - so stdlib macros that
            // call getopts() then go on to use %* for the remaining,
            // non-option arguments (e.g. /echo, /send, /world).
            "getopts" => {
                if args.is_empty() || args.len() > 2 {
                    return Err("getopts requires 1 or 2 arguments (optstring[, init])".to_string());
                }
                let optstring = self.eval(&args[0])?.to_string_value();
                let init = if args.len() == 2 {
                    Some(self.eval(&args[1])?)
                } else {
                    None
                };

                #[derive(Clone, Copy, PartialEq)]
                enum OptKind { Flag, Str, Int, Time }
                let mut spec: Vec<(char, OptKind)> = Vec::new();
                let mut opt_chars = optstring.chars().peekable();
                while let Some(c) = opt_chars.next() {
                    let kind = match opt_chars.peek() {
                        Some(':') => { opt_chars.next(); OptKind::Str }
                        Some('#') => { opt_chars.next(); OptKind::Int }
                        Some('@') => { opt_chars.next(); OptKind::Time }
                        _ => OptKind::Flag,
                    };
                    spec.push((c, kind));
                }

                if let Some(init_val) = &init {
                    for (letter, _) in &spec {
                        self.engine.set_local(&format!("opt_{}", letter), init_val.clone());
                    }
                }

                // Read the calling macro's own positional parameters.
                let argc = self.engine.get_var("#").and_then(|v| v.to_int()).unwrap_or(0).max(0) as usize;
                let mut tokens: Vec<String> = (1..=argc)
                    .map(|i| self.engine.get_var(&i.to_string()).map(|v| v.to_string_value()).unwrap_or_default())
                    .collect();

                let mut consumed = 0usize;
                let mut ok = true;
                while consumed < tokens.len() {
                    let tok = tokens[consumed].clone();
                    // "A '-' or '--' by itself may be used to mark the end
                    // of the options" (`/help options`, verified directly
                    // against real tf 5.0 beta 8) - both forms are
                    // themselves consumed/shifted out of the remaining
                    // args, not just "--". A bare "-" used to fall into the
                    // next arm's `break` with `consumed` left unincremented,
                    // so it stayed in `{*}` as leftover literal text instead
                    // of being shifted away (surfaced by stdlib.tf's own
                    // "/def -i echo = ..." macro - once preloaded, it
                    // shadows Clay's native /echo builtin per finding 16,
                    // so any script calling "/echo -p - text..." - the
                    // stdlib idiom for "a message that itself might start
                    // with '-'" - reached this bug via the macro's own
                    // `getopts("a:poerAw:")` call).
                    if tok == "-" || tok == "--" {
                        consumed += 1;
                        break;
                    }
                    if !tok.starts_with('-') {
                        break;
                    }
                    // Parse the bundled flag cluster within this one token.
                    let mut rest = tok[1..].to_string();
                    loop {
                        let mut it = rest.chars();
                        let c = match it.next() {
                            Some(c) => c,
                            None => break,
                        };
                        let after = it.as_str().to_string();
                        match spec.iter().find(|(letter, _)| *letter == c) {
                            None => {
                                ok = false;
                            }
                            Some((_, OptKind::Flag)) => {
                                self.engine.set_local(&format!("opt_{}", c), TfValue::Integer(1));
                                rest = after;
                                continue;
                            }
                            Some((_, kind)) => {
                                if after.is_empty() {
                                    // No argument attached to this token -
                                    // real tf requires no space between an
                                    // option and its argument.
                                    ok = false;
                                } else {
                                    let value = if *kind == OptKind::Int {
                                        TfValue::from(after.as_str())
                                    } else {
                                        TfValue::String(after)
                                    };
                                    self.engine.set_local(&format!("opt_{}", c), value);
                                }
                            }
                        }
                        break;
                    }
                    consumed += 1;
                    if !ok {
                        break;
                    }
                }

                // Shift: drop the consumed leading tokens (finding C.11).
                if consumed > 0 {
                    tokens.drain(0..consumed);
                    let new_argc = tokens.len();
                    for (i, tok) in tokens.iter().enumerate() {
                        self.engine.set_local(&(i + 1).to_string(), TfValue::String(tok.clone()));
                    }
                    for i in (new_argc + 1)..=argc {
                        self.engine.set_local(&i.to_string(), TfValue::String(String::new()));
                    }
                    self.engine.set_local("#", TfValue::Integer(new_argc as i64));
                    self.engine.set_local("*", TfValue::String(tokens.join(" ")));
                }

                // Real tf prints an error message and returns 0 on a bad
                // option; Clay skips the message (no macro-name context is
                // available here) but preserves the "return 0, don't raise
                // an expression error" contract callers rely on
                // (e.g. "/if (!getopts(...)) /return 0%; /endif").
                Ok(TfValue::Integer(if ok { 1 } else { 0 }))
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
                            "password" | "pass" => TfValue::String(w.password.clone()),
                            "login" => TfValue::Integer(if w.is_connected { 1 } else { 0 }),
                            "ssl" | "secure" => TfValue::Integer(if w.use_ssl { 1 } else { 0 }),
                            // `/addworld ... [<file>]` (finding 31): engine memory only, keyed
                            // by lower-cased world name, falling back to DEFAULT's own file
                            // the same way character/password do (see variables.rs).
                            "file" | "mfile" => {
                                let own = self.engine.world_files.get(&w.name.to_lowercase()).cloned();
                                TfValue::String(own.or_else(|| self.engine.default_world_file.clone()).unwrap_or_default())
                            }
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

            // nactive([world]) - count active worlds, or unseen lines for named world
            "nactive" => {
                if args.is_empty() {
                    let count = self.engine.world_info_cache.iter()
                        .filter(|w| w.is_connected)
                        .count();
                    Ok(TfValue::Integer(count as i64))
                } else {
                    let name = self.eval(&args[0])?.to_string_value();
                    let unseen = self.engine.world_info_cache.iter()
                        .find(|w| w.name.eq_ignore_ascii_case(&name))
                        .map(|w| w.unseen_lines as i64)
                        .unwrap_or(-1);
                    Ok(TfValue::Integer(unseen))
                }
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

            // idle([world]) - seconds since last text received on world
            "idle" => {
                if args.is_empty() {
                    let secs = self.engine.world_info_cache.iter()
                        .find(|w| Some(&w.name) == self.engine.current_world.as_ref())
                        .and_then(|w| w.last_receive_secs_ago)
                        .unwrap_or(0);
                    Ok(TfValue::Integer(secs))
                } else {
                    let name = self.eval(&args[0])?.to_string_value();
                    let secs = self.engine.world_info_cache.iter()
                        .find(|w| w.name.eq_ignore_ascii_case(&name))
                        .and_then(|w| w.last_receive_secs_ago);
                    Ok(TfValue::Integer(secs.unwrap_or(-1)))
                }
            }

            // sidle([world]) - seconds since last text sent to world
            "sidle" => {
                if args.is_empty() {
                    let secs = self.engine.world_info_cache.iter()
                        .find(|w| Some(&w.name) == self.engine.current_world.as_ref())
                        .and_then(|w| w.last_send_secs_ago)
                        .unwrap_or(0);
                    Ok(TfValue::Integer(secs))
                } else {
                    let name = self.eval(&args[0])?.to_string_value();
                    let secs = self.engine.world_info_cache.iter()
                        .find(|w| w.name.eq_ignore_ascii_case(&name))
                        .and_then(|w| w.last_send_secs_ago);
                    Ok(TfValue::Integer(secs.unwrap_or(-1)))
                }
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
            // ftime([format [, time]]) - format a system time (finding
            // C.11's "one-argument ftime" gap plus a few of TF's own
            // format specifiers). One argument formats *now*; the time
            // argument, if given, is whatever mktime()/time() produced
            // (Integer or Float - a fractional part becomes sub-second
            // digits for "%." / "%@"). Formatting uses *local* time, like
            // real tf's own ftime() and like `mktime()` above.
            "ftime" => {
                if args.is_empty() || args.len() > 2 {
                    return Err("ftime requires 1 or 2 arguments (format[, time])".to_string());
                }
                let format = self.eval(&args[0])?.to_string_value();
                let (epoch_secs, frac_secs): (i64, f64) = if args.len() == 2 {
                    let t = self.eval(&args[1])?.to_float().unwrap_or(0.0);
                    (t.floor() as i64, t - t.floor())
                } else {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default();
                    (now.as_secs() as i64, now.subsec_nanos() as f64 / 1_000_000_000.0)
                };

                if format == "@" {
                    return Ok(TfValue::String(format!(
                        "{}.{:06}", epoch_secs, (frac_secs * 1_000_000.0).round() as i64
                    )));
                }

                let lt = crate::util::local_time_from_epoch(epoch_secs);
                Ok(TfValue::String(format_tf_time(&lt, epoch_secs, frac_secs, &format)))
            }

            // fwrite(filename, text) - append text to file
            "fwrite" => {
                if args.len() != 2 {
                    return Err("fwrite requires 2 arguments (filename, text)".to_string());
                }
                let filename = self.eval(&args[0])?.to_string_value();
                let text = self.eval(&args[1])?.to_string_value();

                // Expand ~ in filename
                let expanded = if let Some(rest) = filename.strip_prefix("~/") {
                    if let Some(home) = std::env::var_os("HOME") {
                        format!("{}/{}", home.to_string_lossy(), rest)
                    } else {
                        filename
                    }
                } else {
                    filename
                };

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
                let insert_mode = self.engine.insert_mode();
                self.engine.pending_keyboard_ops.push(super::PendingKeyboardOp::Insert(text, insert_mode));
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

                // Open the file in the specified mode and keep the handle
                let opened_file = match mode {
                    super::TfFileMode::Read => {
                        std::fs::File::open(&filename).ok()
                    }
                    super::TfFileMode::Write => {
                        std::fs::File::create(&filename).ok()
                    }
                    super::TfFileMode::Append => {
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&filename)
                            .ok()
                    }
                };

                let opened_file = match opened_file {
                    Some(f) => f,
                    None => return Ok(TfValue::Integer(0)), // Return 0 on failure (TF convention)
                };

                // Allocate a handle
                let handle = self.engine.next_file_handle;
                self.engine.next_file_handle += 1;

                self.engine.open_files.insert(handle, super::TfFileHandle {
                    path: filename,
                    mode,
                    read_position: 0,
                    file: Some(opened_file),
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

                // Use the stored file handle
                let file = match file_handle.file.as_mut() {
                    Some(f) => f,
                    None => return Ok(TfValue::Integer(0)),
                };

                use std::io::Seek;
                if file.seek(std::io::SeekFrom::Start(file_handle.read_position)).is_err() {
                    return Ok(TfValue::Integer(0));
                }

                // Read one line using a temporary BufReader
                // We need to track bytes read manually
                let mut buf_reader = BufReader::new(file.try_clone().unwrap_or_else(|_| {
                    std::fs::File::open(&file_handle.path).unwrap()
                }));
                if buf_reader.seek(std::io::SeekFrom::Start(file_handle.read_position)).is_err() {
                    return Ok(TfValue::Integer(0));
                }

                let mut line = String::new();
                match buf_reader.read_line(&mut line) {
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

                let file_handle = match self.engine.open_files.get_mut(&handle) {
                    Some(fh) if fh.mode == super::TfFileMode::Write || fh.mode == super::TfFileMode::Append => fh,
                    _ => return Ok(TfValue::Integer(0)),
                };

                // Use the stored file handle
                let file = match file_handle.file.as_mut() {
                    Some(f) => f,
                    None => return Ok(TfValue::Integer(0)),
                };

                // Write with newline
                match writeln!(file, "{}", text) {
                    Ok(_) => Ok(TfValue::Integer(1)),
                    Err(_) => Ok(TfValue::Integer(0)),
                }
            }

            // tfflush(handle [,autoflush]) - flush file (returns 1 on success, 0 on failure)
            // 2nd arg controls auto-flushing (no-op in our implementation)
            "tfflush" => {
                if args.is_empty() || args.len() > 2 {
                    return Err("tfflush requires 1 or 2 arguments (handle [,autoflush])".to_string());
                }
                let handle = self.eval(&args[0])?.to_int().unwrap_or(-1) as i32;

                // 2nd arg (autoflush on/off) is accepted but ignored - we always flush immediately

                // Flush the stored file handle
                match self.engine.open_files.get_mut(&handle) {
                    Some(fh) => {
                        if let Some(ref mut f) = fh.file {
                            let _ = f.flush();
                        }
                        Ok(TfValue::Integer(1))
                    }
                    None => Ok(TfValue::Integer(0)),
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

            // echo(s [,attrs [,dest [,inline]]]) - function form of /echo
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

            // send(s [,world [,flags]]) - function form of /send
            // flags: 0 or "off" = don't append EOL
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
                let no_eol = if args.len() > 2 {
                    let f = self.eval(&args[2])?.to_string_value();
                    f == "0" || f.eq_ignore_ascii_case("off")
                } else {
                    false
                };
                self.engine.pending_commands.push(super::TfCommand {
                    command: text,
                    world,
                    no_eol,
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

            // tr(domain, range, string) - translate characters
            "tr" => {
                if args.len() != 3 {
                    return Err("tr requires 3 arguments (domain, range, string)".to_string());
                }
                let domain_str = self.eval(&args[0])?.to_string_value();
                let range_str = self.eval(&args[1])?.to_string_value();
                let string = self.eval(&args[2])?.to_string_value();
                let domain: Vec<char> = domain_str.chars().collect();
                let range: Vec<char> = range_str.chars().collect();
                Ok(TfValue::String(super::builtins::tr_translate(&domain, &range, &string)))
            }

            // read() - obsolete, use tfread() instead
            "read" => {
                Err("read() is obsolete. Use tfread() instead.".to_string())
            }

            // features([name]) - list, or test, TF's optional build features
            // (finding C.11). Verified directly against real tf 5.0 beta 8's
            // own `/features` output: this build has every feature on
            // except "core" (crash core-dumping) and "SOCKS" (proxy
            // support), which Clay doesn't have either. Case-insensitive,
            // per `/help features`.
            "features" => {
                let (order, off) = features_table();
                if args.is_empty() {
                    let parts: Vec<String> = order.iter().map(|(key, disp)| {
                        let on = !off.contains(key);
                        format!("{}{}", if on { "+" } else { "-" }, disp)
                    }).collect();
                    Ok(TfValue::String(parts.join(" ")))
                } else if args.len() == 1 {
                    let name = self.eval(&args[0])?.to_string_value().to_lowercase();
                    let known = order.iter().any(|(key, _)| *key == name);
                    let on = known && !off.contains(&name.as_str());
                    Ok(TfValue::Integer(if on { 1 } else { 0 }))
                } else {
                    Err("features requires 0 or 1 arguments".to_string())
                }
            }

            // mktime(year [,month [,day [,hour [,minute [,second [,usec]]]]]])
            // - epoch seconds for a date/time in the local time zone
            // (finding C.11). Omitted month/day default to 1, other omitted
            // fields default to 0, matching `/help mktime`. Out-of-range
            // fields (month 13, day 32, ...) are normalized the same way a
            // real mktime(3) - and hence real tf's own mktime(), a thin
            // wrapper around it - does.
            "mktime" => {
                if args.is_empty() || args.len() > 7 {
                    return Err("mktime requires 1 to 7 arguments (year[, month[, day[, hour[, minute[, second[, usec]]]]]])".to_string());
                }
                let year = self.eval(&args[0])?.to_int().unwrap_or(1970);
                let month = if args.len() > 1 { self.eval(&args[1])?.to_int().unwrap_or(1) } else { 1 };
                let day = if args.len() > 2 { self.eval(&args[2])?.to_int().unwrap_or(1) } else { 1 };
                let hour = if args.len() > 3 { self.eval(&args[3])?.to_int().unwrap_or(0) } else { 0 };
                let minute = if args.len() > 4 { self.eval(&args[4])?.to_int().unwrap_or(0) } else { 0 };
                let second = if args.len() > 5 { self.eval(&args[5])?.to_int().unwrap_or(0) } else { 0 };
                let usec = if args.len() > 6 { self.eval(&args[6])?.to_int().unwrap_or(0) } else { 0 };

                let epoch = crate::util::epoch_from_local_time(year, month, day, hour, minute, second);
                if usec != 0 {
                    Ok(TfValue::Float(epoch as f64 + usec as f64 / 1_000_000.0))
                } else {
                    Ok(TfValue::Integer(epoch))
                }
            }

            // cputime() - process CPU time in seconds, or -1 if unavailable.
            "cputime" => {
                if !args.is_empty() {
                    return Err("cputime requires 0 arguments".to_string());
                }
                Ok(TfValue::Float(process_cpu_time_secs()))
            }

            // ln(n) - natural logarithm (finding C.11; real tf also exposes
            // this as log()/log10(), already implemented elsewhere).
            "ln" => {
                if args.len() != 1 {
                    return Err("ln requires 1 argument".to_string());
                }
                let n = self.eval(&args[0])?.to_float().unwrap_or(0.0);
                if n <= 0.0 {
                    return Err("ln: argument must be positive".to_string());
                }
                Ok(TfValue::Float(n.ln()))
            }

            // morepaused([world]) - 1 if the world's output is paused by
            // more-mode or `/dokey pause` (finding C.11). Defined in terms
            // of moresize(), which Clay's engine always reports as 0 (more-
            // mode paging state lives in the main App, not TfEngine) - so
            // this always reports "not paused" too, matching functions.tf's
            // own expectation.
            "morepaused" => {
                if args.len() > 1 {
                    return Err("morepaused requires 0 or 1 arguments".to_string());
                }
                if !args.is_empty() {
                    let _ = self.eval(&args[0])?;
                }
                Ok(TfValue::Integer(0))
            }

            // winlines() - output window height (finding C.11): lines()
            // minus the status/input rows Clay reserves at the bottom of
            // the screen. lines() itself is a fixed default of 24 (see its
            // own arm above - Clay doesn't track real terminal height in
            // TfEngine).
            "winlines" => {
                if !args.is_empty() {
                    return Err("winlines requires 0 arguments".to_string());
                }
                Ok(TfValue::Integer(22))
            }

            // strip_attr(s) - remove all display attributes (finding C.11).
            "strip_attr" => {
                if args.len() != 1 {
                    return Err("strip_attr requires 1 argument".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                Ok(TfValue::String(super::parser::strip_all_attributes(&s)))
            }

            // decode_attr(s1 [, s2 [, f]]) - interpret "@{attr}" codes in s1
            // as display attributes, the same way /echo -p does (finding
            // C.11). Clay represents the result pragmatically as plain text
            // with the attributes encoded directly as embedded ANSI SGR
            // escapes, rather than a true out-of-band attribute channel -
            // see `strlen()`'s doc comment for why that matters. If s2 is
            // given, its attributes (a comma-separated list, as in /echo
            // -a<attrs>) are applied to the whole string; if f is given and
            // falsy, "@{...}" codes in s1 are NOT interpreted (useful for
            // applying only s2's attributes).
            "decode_attr" => {
                if args.is_empty() || args.len() > 3 {
                    return Err("decode_attr requires 1 to 3 arguments (s1 [, s2 [, f]])".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                let interpret = if args.len() >= 3 {
                    self.eval(&args[2])?.to_bool()
                } else {
                    true
                };
                let mut decoded = if interpret {
                    super::parser::process_attr_codes(&s)
                } else {
                    s
                };
                if args.len() >= 2 {
                    let attrs = self.eval(&args[1])?.to_string_value();
                    let prefix = super::parser::attrs_to_ansi_prefix(&attrs);
                    if !prefix.is_empty() {
                        decoded = format!("{}{}\x1b[0m", prefix, decoded);
                    }
                }
                Ok(TfValue::String(decoded))
            }

            // encode_attr(s) - inverse of decode_attr(): re-encode Clay's
            // ANSI-embedded attributed-string representation as "@{attr}"
            // markup text (finding C.11).
            "encode_attr" => {
                if args.len() != 1 {
                    return Err("encode_attr requires 1 argument".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                Ok(TfValue::String(super::parser::encode_attr(&s)))
            }

            // decode_ansi(s) / encode_ansi(s) - convert between raw
            // terminal ANSI attribute codes and the internal attributed-
            // string representation (finding C.11). Since Clay's pragmatic
            // representation of an attributed string already *is* plain
            // text with embedded ANSI SGR escapes (see decode_attr's doc
            // comment), converting between "raw ANSI" and "internal form"
            // is a no-op in both directions for Clay - decode_ansi() is
            // exact identity, and encode_ansi() only has real work to do
            // when its input still has undecoded "@{...}" markup (e.g. text
            // that was never passed through decode_attr()), which it
            // decodes exactly like decode_attr() would.
            "decode_ansi" => {
                if args.len() != 1 {
                    return Err("decode_ansi requires 1 argument".to_string());
                }
                Ok(TfValue::String(self.eval(&args[0])?.to_string_value()))
            }
            "encode_ansi" => {
                if args.len() != 1 {
                    return Err("encode_ansi requires 1 argument".to_string());
                }
                let s = self.eval(&args[0])?.to_string_value();
                Ok(TfValue::String(super::parser::process_attr_codes(&s)))
            }

            // strcmpattr(s1, s2) - like strcmp(), but strings must also
            // match in attributes to be considered equal (finding C.11).
            // Since Clay embeds attributes directly in the text (see
            // decode_attr's doc comment), an ordinary string comparison
            // already differs whenever either the visible text or its
            // attributes differ - exactly strcmpattr's contract - so this
            // needs no extra encode/decode step.
            "strcmpattr" => {
                if args.len() != 2 {
                    return Err("strcmpattr requires 2 arguments".to_string());
                }
                let a = self.eval(&args[0])?.to_string_value();
                let b = self.eval(&args[1])?.to_string_value();
                Ok(TfValue::Integer(a.cmp(&b) as i64))
            }

            // is_open([world]) - whether a world's socket is open (finding
            // C.11). Clay doesn't distinguish "socket open but not yet
            // connected" from "connected" the way real tf can, so this is a
            // pragmatic alias for is_connected() - see that arm above.
            "is_open" => {
                if args.len() > 1 {
                    return Err("is_open requires 0 or 1 arguments".to_string());
                }
                let open = if args.is_empty() {
                    let current = self.engine.current_world.clone().unwrap_or_default();
                    self.engine.world_info_cache.iter()
                        .find(|w| w.name == current)
                        .map(|w| w.is_connected)
                        .unwrap_or(false)
                } else {
                    let name = self.eval(&args[0])?.to_string_value();
                    self.engine.world_info_cache.iter()
                        .find(|w| w.name.eq_ignore_ascii_case(&name))
                        .map(|w| w.is_connected)
                        .unwrap_or(false)
                };
                Ok(TfValue::Integer(if open { 1 } else { 0 }))
            }

            // gethostname() - the local host's name (finding C.11).
            "gethostname" => {
                if !args.is_empty() {
                    return Err("gethostname requires 0 arguments".to_string());
                }
                Ok(TfValue::String(get_hostname()))
            }

            // status_fields([i]) - fields of status row i (finding C.11).
            // Clay has no configurable status bar, so this always reports
            // an empty list, matching functions.tf's own expectation.
            "status_fields" => {
                if args.len() > 1 {
                    return Err("status_fields requires 0 or 1 arguments".to_string());
                }
                if !args.is_empty() {
                    let _ = self.eval(&args[0])?;
                }
                Ok(TfValue::String(String::new()))
            }

            _ => {
                // Try calling as a user-defined macro
                let macro_def = self.engine.macros.iter()
                    .find(|m| m.name.eq_ignore_ascii_case(name))
                    .cloned();
                if let Some(macro_def) = macro_def {
                    // Evaluate all args to strings for positional params
                    let mut arg_strs: Vec<String> = Vec::new();
                    for arg in args {
                        arg_strs.push(self.eval(arg)?.to_string_value());
                    }
                    let arg_refs: Vec<&str> = arg_strs.iter().map(|s| s.as_str()).collect();
                    // Called as a function (`name(args)`), not a command - a
                    // /result in the body must behave exactly like /return
                    // (no echo), per execute_macro_with_context's doc comment.
                    let results = super::macros::execute_macro_with_context(
                        self.engine, &macro_def, &arg_refs, None, true,
                    );
                    // Process results: collect messages, handle Return/Result.
                    // Neither variant is actually pushed into `results` by
                    // execute_macro (it sets %? and breaks instead - see
                    // there), so the real propagation path is the `?`
                    // fallback below; these arms are kept for symmetry/
                    // robustness in case that ever changes.
                    let mut return_val = None;
                    for result in &results {
                        match result {
                            super::TfCommandResult::Return(val) | super::TfCommandResult::Result(val) => {
                                return_val = Some(TfValue::from(val.as_str()));
                            }
                            super::TfCommandResult::Error(e) => {
                                self.engine.set_global("?", TfValue::Integer(0));
                                return Err(e.clone());
                            }
                            _ => {}
                        }
                    }
                    if let Some(val) = return_val {
                        self.engine.set_global("?", val.clone());
                        Ok(val)
                    } else {
                        Ok(self.engine.get_var("?").cloned().unwrap_or(TfValue::Integer(1)))
                    }
                } else if super::parser::is_tf_command_name(name) {
                    // Try calling as a builtin command
                    let mut arg_strs: Vec<String> = Vec::new();
                    for arg in args {
                        arg_strs.push(self.eval(arg)?.to_string_value());
                    }
                    let cmd = format!("/{} {}", name, arg_strs.join(" "));
                    let result = self.engine.execute(&cmd);
                    match result {
                        super::TfCommandResult::Success(_) => {
                            Ok(self.engine.get_var("?").cloned().unwrap_or(TfValue::Integer(1)))
                        }
                        super::TfCommandResult::Error(e) => {
                            self.engine.set_global("?", TfValue::Integer(0));
                            Err(e)
                        }
                        _ => Ok(TfValue::Integer(1))
                    }
                } else {
                    Err(format!("Unknown function: {}", name))
                }
            }
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

    let char_len = s.chars().count();
    if width == 0 || char_len >= width {
        return s.to_string();
    }

    let padding = width - char_len;
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

/// The `features()` expression function's table, shared with the `/features` COMMAND
/// (`builtins::cmd_features`, Job 15) so the two can never drift apart. Verified
/// directly against real tf 5.0 beta 8's own `/features` output: this build has every
/// feature on except "core" (crash core-dumping) and "SOCKS" (proxy support), which
/// Clay doesn't have either. Returns (name/display-name pairs in tf's own order, the
/// subset of names that are off).
pub fn features_table() -> (&'static [(&'static str, &'static str)], &'static [&'static str]) {
    const ORDER: [(&str, &str); 14] = [
        ("256colors", "256colors"), ("core", "core"), ("float", "float"),
        ("ftime", "ftime"), ("history", "history"), ("ipv6", "IPv6"),
        ("locale", "locale"), ("mccpv1", "MCCPv1"), ("mccpv2", "MCCPv2"),
        ("process", "process"), ("socks", "SOCKS"), ("ssl", "ssl"),
        ("subsecond", "subsecond"), ("tz", "TZ"),
    ];
    const OFF: [&str; 2] = ["core", "socks"];
    (&ORDER, &OFF)
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

/// Process CPU time (user + system) in seconds, for the `cputime()`
/// function. Matches real tf's own documented fallback of -1 when
/// unavailable.
#[cfg(unix)]
pub(crate) fn process_cpu_time_secs() -> f64 {
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        if libc::getrusage(0 /* RUSAGE_SELF */, &mut usage) == 0 {
            let user = usage.ru_utime.tv_sec as f64 + usage.ru_utime.tv_usec as f64 / 1_000_000.0;
            let sys = usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1_000_000.0;
            user + sys
        } else {
            -1.0
        }
    }
}
#[cfg(not(unix))]
pub(crate) fn process_cpu_time_secs() -> f64 {
    -1.0
}

/// The local host's name, for the `gethostname()` function. Real tf
/// returns an empty string if the host name isn't available; Clay does the
/// same on any error or on a platform without a hostname syscall binding.
#[cfg(unix)]
fn get_hostname() -> String {
    let mut buf = [0u8; 256];
    unsafe {
        if libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            return String::from_utf8_lossy(&buf[..end]).into_owned();
        }
    }
    String::new()
}
#[cfg(not(unix))]
fn get_hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

/// Format a local time for the `ftime()` function, supporting real TF's
/// documented specifiers that are cheap to add given `crate::util::LocalTime`
/// (see `/help ftime` for the full table) plus its two nonstandard
/// subsecond extensions, `%@` (raw epoch, to the microsecond) and `%.`
/// (microseconds since the last whole second) - both needed by tf-lib's
/// at.tf (`ftime("%F %T.%.", t)`). Unknown specifiers pass through
/// literally, matching `util::format_local_time`'s own convention.
pub(crate) fn format_tf_time(lt: &crate::util::LocalTime, epoch_secs: i64, frac_secs: f64, fmt: &str) -> String {
    const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December",
    ];
    let usec = (frac_secs * 1_000_000.0).round() as i64;
    let weekday = (lt.weekday.rem_euclid(7)) as usize;
    let month_idx = ((lt.month - 1).rem_euclid(12)) as usize;

    let mut result = String::with_capacity(fmt.len() + 8);
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => result.push_str(&format!("{:04}", lt.year)),
            Some('y') => result.push_str(&format!("{:02}", lt.year.rem_euclid(100))),
            Some('m') => result.push_str(&format!("{:02}", lt.month)),
            Some('d') => result.push_str(&format!("{:02}", lt.day)),
            Some('H') => result.push_str(&format!("{:02}", lt.hour)),
            Some('I') => {
                let h12 = match lt.hour % 12 { 0 => 12, h => h };
                result.push_str(&format!("{:02}", h12));
            }
            Some('M') => result.push_str(&format!("{:02}", lt.minute)),
            Some('S') => result.push_str(&format!("{:02}", lt.second)),
            Some('p') => result.push_str(if lt.hour < 12 { "AM" } else { "PM" }),
            Some('a') => result.push_str(&WEEKDAYS[weekday][..3]),
            Some('A') => result.push_str(WEEKDAYS[weekday]),
            Some('b') => result.push_str(&MONTHS[month_idx][..3]),
            Some('B') => result.push_str(MONTHS[month_idx]),
            Some('F') => result.push_str(&format!("{:04}-{:02}-{:02}", lt.year, lt.month, lt.day)),
            Some('T') => result.push_str(&format!("{:02}:{:02}:{:02}", lt.hour, lt.minute, lt.second)),
            Some('j') => result.push_str(&format!("{:03}", day_of_year(lt))),
            Some('w') => result.push_str(&weekday.to_string()),
            Some('s') => result.push_str(&epoch_secs.to_string()),
            Some('@') => result.push_str(&format!("{}.{:06}", epoch_secs, usec)),
            Some('.') => result.push_str(&format!("{:06}", usec)),
            Some('%') => result.push('%'),
            Some(x) => { result.push('%'); result.push(x); }
            None => result.push('%'),
        }
    }
    result
}

/// 1-based day of year, for ftime()'s "%j".
fn day_of_year(lt: &crate::util::LocalTime) -> i32 {
    const CUM_DAYS: [i32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = (lt.year % 4 == 0 && lt.year % 100 != 0) || lt.year % 400 == 0;
    let month_idx = ((lt.month - 1).clamp(0, 11)) as usize;
    let mut days = CUM_DAYS[month_idx] + lt.day;
    if leap && lt.month > 2 {
        days += 1;
    }
    days
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
        let mut lexer = Lexer::new("42 3.25 1e10");
        assert_eq!(lexer.next_token().unwrap(), Token::Integer(42));
        assert_eq!(lexer.next_token().unwrap(), Token::Float(3.25));
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

    /// Set up positional parameters "a b c d" (#=4) the same way
    /// execute_macro does, for the "{...}" selector tests below.
    fn set_args(engine: &mut TfEngine, args: &[&str]) {
        engine.set_global("#", TfValue::Integer(args.len() as i64));
        for (i, arg) in args.iter().enumerate() {
            engine.set_global(&(i + 1).to_string(), TfValue::String(arg.to_string()));
        }
    }

    #[test]
    fn test_eval_brace_selectors() {
        // {n} and {#} were already correct; {-1} used to compute "the last
        // argument" (TF's "L1" meaning) instead of its real meaning, "all
        // but the first" - verified directly against real tf 5.0 beta 8
        // (see resolve_extended_selector's doc comment). {L}/{LN} and
        // {-L}/{-LN} were entirely unimplemented (bare {L} resolved to an
        // empty, undefined variable named "L").
        let mut engine = TfEngine::new();
        set_args(&mut engine, &["a", "b", "c", "d"]);

        assert_eq!(evaluate(&mut engine, "{1}").unwrap(), TfValue::String("a".to_string()));
        assert_eq!(evaluate(&mut engine, "{#}").unwrap(), TfValue::Integer(4));

        assert_eq!(evaluate(&mut engine, "{-1}").unwrap(), TfValue::String("b c d".to_string()));
        assert_eq!(evaluate(&mut engine, "{-2}").unwrap(), TfValue::String("c d".to_string()));

        assert_eq!(evaluate(&mut engine, "{L}").unwrap(), TfValue::String("d".to_string()));
        assert_eq!(evaluate(&mut engine, "{L2}").unwrap(), TfValue::String("c".to_string()));

        assert_eq!(evaluate(&mut engine, "{-L}").unwrap(), TfValue::String("a b c".to_string()));
        assert_eq!(evaluate(&mut engine, "{-L2}").unwrap(), TfValue::String("a b".to_string()));
    }

    #[test]
    fn test_eval_comma_operator() {
        // "e1, e2" - TF's lowest-precedence operator: evaluate both, left
        // to right, keep e2's value (/help expressions' operator table).
        // Entirely unimplemented before this job - needed by real tf-lib
        // idioms like "/while (shift(), {#}) ..." and
        // "/test (result:=result*n), --n".
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "(x := 1), (y := 2)").unwrap(), TfValue::Integer(2));
        assert_eq!(engine.get_var("x").and_then(|v| v.to_int()), Some(1));
        assert_eq!(engine.get_var("y").and_then(|v| v.to_int()), Some(2));

        // A bare (unparenthesized) top-level comma also works - this is
        // how a /if//while//for condition's own text reaches evaluate()
        // (the condition parser strips only the outer parens).
        assert_eq!(evaluate(&mut engine, "z := 5, z > 3").unwrap(), TfValue::Integer(1));

        // Comma inside a function call's argument list is still the
        // argument separator, NOT the sequencing operator - min(1,2,3)
        // must still receive three arguments, not one comma-chained one.
        assert_eq!(evaluate(&mut engine, "min(3, 1, 4)").unwrap(), TfValue::Integer(1));
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

    // =========================================================================
    // Job 11: functions from finding C.11, replace()'s TF argument order,
    // finding 20's `:=` scope rule.
    // =========================================================================

    #[test]
    fn test_features_function() {
        let mut engine = TfEngine::new();
        // Verified against real tf 5.0 beta 8's own /features output.
        for on in ["256colors", "float", "ftime", "history", "IPv6", "locale",
                   "MCCPv1", "MCCPv2", "process", "ssl", "subsecond", "TZ"] {
            assert_eq!(
                evaluate(&mut engine, &format!("features(\"{}\")", on)).unwrap(),
                TfValue::Integer(1),
                "feature {} should be on",
                on
            );
        }
        for off in ["core", "SOCKS"] {
            assert_eq!(
                evaluate(&mut engine, &format!("features(\"{}\")", off)).unwrap(),
                TfValue::Integer(0),
                "feature {} should be off",
                off
            );
        }
        // Case-insensitive, per /help features.
        assert_eq!(evaluate(&mut engine, "features(\"SSL\")").unwrap(), TfValue::Integer(1));
        // Unknown name: 0, not an error.
        assert_eq!(evaluate(&mut engine, "features(\"nope\")").unwrap(), TfValue::Integer(0));
        // No-argument form: "+name -name ..." list.
        let list = evaluate(&mut engine, "features()").unwrap().to_string_value();
        assert!(list.contains("+256colors"), "list: {}", list);
        assert!(list.contains("-core"), "list: {}", list);
        assert!(list.contains("-SOCKS"), "list: {}", list);
    }

    #[test]
    fn test_mktime_function() {
        let mut engine = TfEngine::new();
        // Basic sanity: a date decades after the epoch is a large positive
        // number, and later dates are later epochs.
        let t1 = evaluate(&mut engine, "mktime(2001,9,9,0,0,0)").unwrap().to_int().unwrap();
        assert!(t1 > 0);
        let t2 = evaluate(&mut engine, "mktime(2001,9,10,0,0,0)").unwrap().to_int().unwrap();
        assert!(t2 > t1, "a later date must have a later epoch");
        // Omitted fields default to 1 (month/day) or 0 (hour/minute/second),
        // per /help mktime.
        let t3 = evaluate(&mut engine, "mktime(2001)").unwrap().to_int().unwrap();
        let t4 = evaluate(&mut engine, "mktime(2001,1,1,0,0,0)").unwrap().to_int().unwrap();
        assert_eq!(t3, t4);
        // usec argument produces a Float.
        let t5 = evaluate(&mut engine, "mktime(2001,1,1,0,0,0,500000)").unwrap();
        match t5 {
            TfValue::Float(f) => assert!((f - (t4 as f64 + 0.5)).abs() < 1e-9),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn test_ftime_function() {
        let mut engine = TfEngine::new();
        // 2-argument form, a fixed timestamp - deterministic.
        assert_eq!(
            evaluate(&mut engine, "ftime(\"%Y-%m\", 1000000000)").unwrap(),
            TfValue::String("2001-09".to_string())
        );
        // %F / %T / %. / %@ extensions.
        assert_eq!(
            evaluate(&mut engine, "ftime(\"%F\", 1000000000)").unwrap().to_string_value().len(),
            10 // "YYYY-MM-DD"
        );
        assert_eq!(
            evaluate(&mut engine, "ftime(\"%.\", 1000000000)").unwrap(),
            TfValue::String("000000".to_string())
        );
        let at = evaluate(&mut engine, "ftime(\"@\", 1000000000)").unwrap().to_string_value();
        assert_eq!(at, "1000000000.000000");
        // 1-argument form (format only) = now: just check it doesn't error
        // and produces a 4-digit year.
        let now_year = evaluate(&mut engine, "ftime(\"%Y\")").unwrap().to_string_value();
        assert_eq!(now_year.len(), 4);
    }

    #[test]
    fn test_cputime_function() {
        let mut engine = TfEngine::new();
        let v = evaluate(&mut engine, "cputime()").unwrap().to_float().unwrap();
        assert!(v >= 0.0 || v == -1.0, "cputime should be nonneg or -1: {}", v);
    }

    #[test]
    fn test_ln_function() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "ln(1)").unwrap(), TfValue::Float(0.0));
        let ln2 = evaluate(&mut engine, "ln(2)").unwrap().to_float().unwrap();
        assert!((ln2 - std::f64::consts::LN_2).abs() < 1e-9);
        assert!(evaluate(&mut engine, "ln(-1)").is_err());
        assert!(evaluate(&mut engine, "ln(0)").is_err());
    }

    #[test]
    fn test_morepaused_and_winlines_functions() {
        let mut engine = TfEngine::new();
        // moresize() always reports 0 (more-mode isn't tracked by TfEngine),
        // so morepaused() - defined in terms of it - always reports 0 too.
        assert_eq!(evaluate(&mut engine, "morepaused()").unwrap(), TfValue::Integer(0));
        assert_eq!(evaluate(&mut engine, "morepaused(\"somWorld\")").unwrap(), TfValue::Integer(0));
        let wl = evaluate(&mut engine, "winlines()").unwrap().to_int().unwrap();
        assert!(wl > 0);
    }

    #[test]
    fn test_strip_attr_function() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "strip_attr(\"x\")").unwrap(), TfValue::String("x".to_string()));
        // Raw, undecoded "@{...}" markup is stripped.
        assert_eq!(
            evaluate(&mut engine, "strip_attr(\"@{Cred}hello@{n}\")").unwrap(),
            TfValue::String("hello".to_string())
        );
        // Already-decoded (embedded ANSI) text is stripped too, down to
        // just the visible text.
        let decoded = super::super::parser::process_attr_codes("@{Cred}hi@{n}");
        assert_ne!(decoded, "hi", "decode_attr should have embedded real ANSI");
        assert_eq!(super::super::parser::strip_all_attributes(&decoded), "hi");
    }

    #[test]
    fn test_decode_attr_and_strlen_ignore_attributes() {
        let mut engine = TfEngine::new();
        // cylon.tf's own case: decode_attr() on a string built from
        // "@{Cbgblack}" + six single-space "@{Cbgrgb...} @{}" runs + 7
        // trailing spaces + "@{n}" has exactly 13 *visible* characters -
        // strlen() must not count the embedded ANSI bytes decode_attr()
        // produces (verified end-to-end against real tf via lib_cylon.tf).
        let cylon0 = "@{Cbgblack}@{Cbgrgb100} @{}@{Cbgrgb200} @{}@{Cbgrgb300} @{}@{Cbgrgb400} @{}@{Cbgrgb500} @{}@{Cbgrgb000} @{}       @{n}";
        engine.set_global("cylon0", TfValue::String(cylon0.to_string()));
        let r = evaluate(&mut engine, "cylon0 := decode_attr(cylon0)");
        assert!(r.is_ok(), "{:?}", r);
        assert_eq!(evaluate(&mut engine, "strlen(cylon0)").unwrap(), TfValue::Integer(13));
    }

    #[test]
    fn test_encode_attr_round_trips_decode_attr() {
        // Round-trips exactly for every color-attribute form attr_to_ansi
        // supports (named, bright, 216-cube, grayscale) plus the basic
        // bold/underline/reverse/reset codes - going straight through
        // parser::process_attr_codes/encode_attr avoids needing a real
        // ESC byte inside a Rust string literal.
        for code in ["@{Cbgblack}", "@{Cbgrgb500}", "@{Crgb320}", "@{Cgray10}",
                     "@{Cbggray10}", "@{n}", "@{B}", "@{U}", "@{Cred}", "@{Cbgred}"] {
            let name = &code[2..code.len() - 1];
            let decoded = super::super::parser::process_attr_codes(code);
            assert_ne!(decoded, code, "{} should have been converted to ANSI", code);
            assert_eq!(super::super::parser::encode_attr(&decoded), format!("@{{{}}}", name), "round-trip failed for {}", code);
        }
    }

    #[test]
    fn test_decode_ansi_and_encode_ansi_functions() {
        let mut engine = TfEngine::new();
        // decode_ansi is identity for Clay's representation.
        assert_eq!(
            evaluate(&mut engine, "decode_ansi(\"plain text\")").unwrap(),
            TfValue::String("plain text".to_string())
        );
        // encode_ansi decodes any remaining "@{...}" markup.
        let r = evaluate(&mut engine, "encode_ansi(\"@{n}x\")").unwrap().to_string_value();
        assert!(r.ends_with('x'));
        assert_ne!(r, "@{n}x");
    }

    #[test]
    fn test_strcmpattr_function() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "strcmpattr(\"a\", \"a\")").unwrap(), TfValue::Integer(0));
        assert_ne!(evaluate(&mut engine, "strcmpattr(\"a\", \"b\")").unwrap(), TfValue::Integer(0));
    }

    #[test]
    fn test_is_open_and_gethostname_and_status_fields_functions() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "is_open()").unwrap(), TfValue::Integer(0));
        assert_eq!(evaluate(&mut engine, "is_open(\"noworld\")").unwrap(), TfValue::Integer(0));
        // gethostname() should at least not error; content is host-dependent.
        assert!(evaluate(&mut engine, "gethostname()").is_ok());
        assert_eq!(evaluate(&mut engine, "status_fields()").unwrap(), TfValue::String(String::new()));
        assert_eq!(evaluate(&mut engine, "status_fields(0)").unwrap(), TfValue::String(String::new()));
    }

    #[test]
    fn test_replace_function_tf_argument_order() {
        let mut engine = TfEngine::new();
        // TF order: replace(old, new, str) - "a" -> "o" in "banana" = "bonono".
        assert_eq!(
            evaluate(&mut engine, "replace(\"a\", \"o\", \"banana\")").unwrap(),
            TfValue::String("bonono".to_string())
        );
        // Clay's optional 4th `count` argument still works in the new
        // order: replace only the first occurrence.
        assert_eq!(
            evaluate(&mut engine, "replace(\"a\", \"o\", \"banana\", 1)").unwrap(),
            TfValue::String("bonana".to_string())
        );
    }

    #[test]
    fn test_getopts_two_arg_form_with_flags_and_shift() {
        let mut engine = TfEngine::new();
        engine.push_scope();
        engine.set_local("#", TfValue::Integer(3));
        engine.set_local("1", TfValue::String("-v".to_string()));
        engine.set_local("2", TfValue::String("rest1".to_string()));
        engine.set_local("3", TfValue::String("rest2".to_string()));
        engine.set_local("*", TfValue::String("-v rest1 rest2".to_string()));

        let r = evaluate(&mut engine, "getopts(\"v\", \"\")").unwrap();
        assert_eq!(r, TfValue::Integer(1));
        assert_eq!(engine.get_var("opt_v"), Some(&TfValue::Integer(1)));
        // The consumed "-v" token is shifted out of the positional params.
        assert_eq!(engine.get_var("#"), Some(&TfValue::Integer(2)));
        assert_eq!(engine.get_var("1").map(|v| v.to_string_value()), Some("rest1".to_string()));
        assert_eq!(engine.get_var("2").map(|v| v.to_string_value()), Some("rest2".to_string()));
        assert_eq!(engine.get_var("*").map(|v| v.to_string_value()), Some("rest1 rest2".to_string()));
        engine.pop_scope();
    }

    #[test]
    fn test_getopts_one_arg_form_no_options_present() {
        let mut engine = TfEngine::new();
        engine.push_scope();
        engine.set_local("#", TfValue::Integer(1));
        engine.set_local("1", TfValue::String("badtime".to_string()));
        engine.set_local("*", TfValue::String("badtime".to_string()));
        // 1-argument form: no init value given.
        let r = evaluate(&mut engine, "getopts(\"v\")").unwrap();
        assert_eq!(r, TfValue::Integer(1), "no '-' token is not an error");
        assert_eq!(engine.get_var("opt_v"), None, "opt_v was never set and had no init value");
        // Nothing consumed - args unchanged.
        assert_eq!(engine.get_var("#"), Some(&TfValue::Integer(1)));
        assert_eq!(engine.get_var("1").map(|v| v.to_string_value()), Some("badtime".to_string()));
        engine.pop_scope();
    }

    #[test]
    fn test_getopts_init_value_and_numeric_suffix() {
        let mut engine = TfEngine::new();
        engine.push_scope();
        engine.set_local("#", TfValue::Integer(1));
        engine.set_local("1", TfValue::String("-n5".to_string()));
        engine.set_local("*", TfValue::String("-n5".to_string()));
        // "n#" - an integer-expression argument, bundled inline.
        let r = evaluate(&mut engine, "getopts(\"an#s:\", \"\")").unwrap();
        assert_eq!(r, TfValue::Integer(1));
        assert_eq!(engine.get_var("opt_n"), Some(&TfValue::Integer(5)));
        // "a" and "s" were declared but not present: initialized to "" by
        // the 2-argument form's <init>.
        assert_eq!(engine.get_var("opt_a"), Some(&TfValue::String(String::new())));
        assert_eq!(engine.get_var("opt_s"), Some(&TfValue::String(String::new())));
        engine.pop_scope();
    }

    #[test]
    fn test_assign_scope_rule_finding_20() {
        // := updates an existing LOCAL binding wherever it lives, updates
        // an existing GLOBAL if there's no local, and otherwise creates a
        // new GLOBAL - never just the innermost local scope.
        let mut engine = TfEngine::new();

        // Neither local nor global exists: creates a global.
        assert_eq!(evaluate(&mut engine, "newvar := 5").unwrap(), TfValue::Integer(5));
        assert_eq!(engine.global_vars.get("newvar"), Some(&TfValue::Integer(5)));
        assert!(engine.local_vars_stack.is_empty());

        // Only a global exists: := updates the global, even from inside a
        // local scope (stack-q.tf's own /push idiom).
        engine.push_scope();
        evaluate(&mut engine, "newvar := 6").unwrap();
        assert_eq!(engine.global_vars.get("newvar"), Some(&TfValue::Integer(6)));
        assert!(
            engine.local_vars_stack.last().unwrap().get("newvar").is_none(),
            "must not have created a local shadow"
        );
        engine.pop_scope();

        // A local binding exists (via /let-equivalent set_local): :=
        // updates THAT binding, not the (nonexistent, or even a
        // pre-existing but different) global.
        engine.set_global("shadowed", TfValue::Integer(100));
        engine.push_scope();
        engine.set_local("shadowed", TfValue::Integer(1));
        evaluate(&mut engine, "shadowed := 2").unwrap();
        assert_eq!(engine.local_vars_stack.last().unwrap().get("shadowed"), Some(&TfValue::Integer(2)));
        assert_eq!(engine.global_vars.get("shadowed"), Some(&TfValue::Integer(100)), "global must be untouched");
        engine.pop_scope();

        // ++ / -- follow the same rule.
        engine.set_global("counter", TfValue::Integer(1));
        engine.push_scope();
        evaluate(&mut engine, "++counter").unwrap();
        assert_eq!(engine.global_vars.get("counter"), Some(&TfValue::Integer(2)));
        engine.pop_scope();
    }

    /// Finding 31 / plan Job 14b: `/addworld ... <name> ... [<file>]`'s per-world script
    /// is kept in `TfEngine::world_files` (never persisted) and read back through
    /// `world_info(name, "file")`, falling back to DEFAULT's own file when the world
    /// has none of its own - same fallback rule as character/password (variables.rs).
    #[test]
    fn test_world_info_file_field_and_default_fallback() {
        use crate::tf::WorldInfoCache;

        let mut engine = TfEngine::new();
        engine.world_info_cache = vec![
            WorldInfoCache { name: "HasFile".to_string(), ..Default::default() },
            WorldInfoCache { name: "NoFile".to_string(), ..Default::default() },
        ];
        engine.world_files.insert("hasfile".to_string(), "/tmp/hasfile.tf".to_string());
        engine.default_world_file = Some("/tmp/default.tf".to_string());

        assert_eq!(
            evaluate(&mut engine, r#"world_info("HasFile", "file")"#).unwrap(),
            TfValue::String("/tmp/hasfile.tf".to_string())
        );
        assert_eq!(
            evaluate(&mut engine, r#"world_info("NoFile", "file")"#).unwrap(),
            TfValue::String("/tmp/default.tf".to_string()),
            "a world with no file of its own falls back to DEFAULT's"
        );
        assert_eq!(
            evaluate(&mut engine, r#"world_info("Unknown", "mfile")"#).unwrap(),
            TfValue::String(String::new()),
            "an unknown world name has no cache entry to fall back through"
        );
    }

    /// Job 15b-i: a ternary's TRUE branch must allow the comma operator
    /// with no parentheses needed - real tf's grammar mirrors C's here
    /// (verified directly against real tf: `/eval /echo $[1 ? 2,3 : 4]`
    /// -> "3", `$[0 ? 2,3 : 4]` -> "4"). `parse_ternary` used to call
    /// `parse_or` for the true branch (comma-operator precedence, the
    /// LOWEST level, was never reached), so stdlib.tf's own "/nth"
    /// one-liner (`/result {1} > 0 ? shift({1}), {1} : ""`) errored
    /// "Expected Colon, got Comma" instead of evaluating the shift for
    /// its side effect and reading the (now-shifted) {1}. The comma
    /// operator must still be excluded from the CONDITION and the FALSE
    /// branch, matching C - only the true branch changed.
    #[test]
    fn test_ternary_true_branch_allows_comma_operator() {
        let mut engine = TfEngine::new();
        assert_eq!(evaluate(&mut engine, "1 ? 2,3 : 4").unwrap(), TfValue::Integer(3));
        assert_eq!(evaluate(&mut engine, "0 ? 2,3 : 4").unwrap(), TfValue::Integer(4));
    }

    /// stdlib.tf's own "/nth" idiom, end to end.
    #[test]
    fn test_ternary_comma_operator_shift_side_effect_idiom() {
        let mut engine = TfEngine::new();
        engine.set_local("1", TfValue::Integer(2));
        engine.set_local("2", TfValue::String("a".to_string()));
        engine.set_local("3", TfValue::String("b".to_string()));
        engine.set_local("4", TfValue::String("c".to_string()));
        engine.set_local("*", TfValue::String("a b c".to_string()));
        engine.set_local("#", TfValue::Integer(3));
        assert_eq!(
            evaluate(&mut engine, r#"{1} > 0 ? shift({1}), {1} : """#).unwrap(),
            TfValue::String("b".to_string())
        );
    }
}
