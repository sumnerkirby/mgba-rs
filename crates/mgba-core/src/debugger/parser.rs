// Copyright (c) 2013-2014 Jeffrey Pfau
// Rust port of mGBA's debugger expression lexer, parser and evaluator
// (src/debugger/parser.c, include/mgba/internal/debugger/parser.h).
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! mGBA's parse tree nodes point at their parent and children; here they live
//! in a vector and refer to each other by index.

/// `enum Operation`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Assign,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    And,
    Or,
    Xor,
    Less,
    Greater,
    Equal,
    NotEqual,
    LogicalAnd,
    LogicalOr,
    Le,
    Ge,
    Negate,
    Flip,
    Not,
    ShiftL,
    ShiftR,
    Dereference,
}

impl Operation {
    /// `_operatorPrecedence`
    fn precedence(self) -> i32 {
        use Operation::*;
        match self {
            Assign => 14,
            Add | Subtract => 4,
            Multiply | Divide | Modulo => 3,
            And => 8,
            Or => 10,
            Xor => 9,
            Less | Greater | Le | Ge => 6,
            Equal | NotEqual => 7,
            LogicalAnd => 11,
            LogicalOr => 12,
            Negate | Flip | Not | Dereference => 2,
            ShiftL | ShiftR => 5,
        }
    }
}

/// `struct Token`
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Error,
    Uint(u32),
    Identifier(String),
    Operator(Operation),
    OpenParen,
    CloseParen,
    Segment(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LexState {
    Error,
    Root,
    ExpectIdentifier,
    ExpectBinaryFirst,
    ExpectBinary,
    ExpectDecimal,
    ExpectHexFirst,
    ExpectHex,
    ExpectPrefix,
    ExpectOperator2,
}

/// `_lexOperator`
fn lex_operator(lv: &mut Vec<Token>, operator: u8, state: &mut LexState) {
    use Operation::*;
    if *state == LexState::ExpectOperator2 {
        let last = lv.last_mut().expect("operator token");
        let Token::Operator(op) = *last else {
            *last = Token::Error;
            *state = LexState::Error;
            return;
        };
        let combined = match (op, operator) {
            (And, b'&') => Some(LogicalAnd),
            (Or, b'|') => Some(LogicalOr),
            (Less, b'=') => Some(Le),
            (Less, b'<') => Some(ShiftL),
            (Greater, b'=') => Some(Ge),
            (Greater, b'>') => Some(ShiftR),
            (Assign, b'=') => Some(Equal),
            (Not, b'=') => Some(NotEqual),
            _ => None,
        };
        if let Some(c) = combined {
            *last = Token::Operator(c);
            *state = LexState::Root;
            return;
        }
        *state = LexState::Error;
    }
    let op = match operator {
        b'=' => Assign,
        b'+' => Add,
        b'-' => Subtract,
        b'*' => Multiply,
        b'/' => Divide,
        b'%' => Modulo,
        b'&' => And,
        b'|' => Or,
        b'^' => Xor,
        b'<' => Less,
        b'>' => Greater,
        b'!' => Not,
        b'~' => Flip,
        _ => {
            *state = LexState::Error;
            return;
        }
    };
    *state = LexState::ExpectOperator2;
    lv.push(Token::Operator(op));
}

/// `_lexValue`
fn lex_value(lv: &mut Vec<Token>, token: u8, next: u32, state: &mut LexState) {
    match token {
        b'=' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'<' | b'>' | b'!' => {
            lv.push(Token::Uint(next));
            lex_operator(lv, token, state);
        }
        b')' => {
            lv.push(Token::Uint(next));
            lv.push(Token::CloseParen);
            *state = LexState::Root;
        }
        b' ' | b'\t' => {
            lv.push(Token::Uint(next));
            *state = LexState::Root;
        }
        _ => *state = LexState::Error,
    }
}

/// `lexExpression`: tokenize `string` up to the first character in `eol`
/// (space, CR and LF by default). Returns how many bytes were consumed.
pub fn lex_expression(lv: &mut Vec<Token>, string: &str, eol: Option<&str>) -> usize {
    let bytes = string.as_bytes();
    if bytes.is_empty() {
        return 0;
    }
    let eol = eol.unwrap_or(" \r\n").as_bytes();
    let mut next: u32 = 0;
    let mut adjusted = 0;
    let mut state = LexState::Root;
    let mut token_start = 0usize;
    let mut pos = 0usize;
    while pos < bytes.len() && bytes[pos] != 0 && !eol.contains(&bytes[pos]) && state != LexState::Error {
        let token = bytes[pos];
        pos += 1;
        adjusted += 1;
        let mut root = false;
        match state {
            LexState::ExpectOperator2 => {
                if matches!(token, b'&' | b'|' | b'=' | b'<' | b'>') {
                    lex_operator(lv, token, &mut state);
                }
                if state == LexState::ExpectOperator2 {
                    root = true;
                }
            }
            LexState::Root => root = true,
            LexState::ExpectIdentifier => match token {
                b'=' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'<' | b'>' | b'!' | b'~' => {
                    lv.push(Token::Identifier(string[token_start..pos - 1].to_string()));
                    lex_operator(lv, token, &mut state);
                }
                b')' => {
                    lv.push(Token::Identifier(string[token_start..pos - 1].to_string()));
                    lv.push(Token::CloseParen);
                    state = LexState::Root;
                }
                b' ' | b'\t' => {
                    lv.push(Token::Identifier(string[token_start..pos - 1].to_string()));
                    state = LexState::Root;
                }
                _ => {}
            },
            LexState::ExpectBinaryFirst | LexState::ExpectBinary => {
                state = LexState::ExpectBinary;
                match token {
                    b'0' | b'1' => next = (next << 1).wrapping_add((token - b'0') as u32),
                    _ => lex_value(lv, token, next, &mut state),
                }
            }
            LexState::ExpectDecimal => match token {
                b'0'..=b'9' => next = next.wrapping_mul(10).wrapping_add((token - b'0') as u32),
                _ => lex_value(lv, token, next, &mut state),
            },
            LexState::ExpectHexFirst | LexState::ExpectHex => {
                state = LexState::ExpectHex;
                match token {
                    b'0'..=b'9' => next = next.wrapping_mul(16).wrapping_add((token - b'0') as u32),
                    b'A'..=b'F' => next = next.wrapping_mul(16).wrapping_add((token - b'A' + 10) as u32),
                    b'a'..=b'f' => next = next.wrapping_mul(16).wrapping_add((token - b'a' + 10) as u32),
                    b':' => {
                        lv.push(Token::Segment(next));
                        next = 0;
                    }
                    _ => lex_value(lv, token, next, &mut state),
                }
            }
            LexState::ExpectPrefix => match token {
                b'X' | b'x' => {
                    next = 0;
                    state = LexState::ExpectHexFirst;
                }
                b'B' | b'b' => {
                    next = 0;
                    state = LexState::ExpectBinaryFirst;
                }
                b'0'..=b'9' => {
                    next = (token - b'0') as u32;
                    state = LexState::ExpectDecimal;
                }
                _ => lex_value(lv, token, next, &mut state),
            },
            LexState::Error => {}
        }
        if root {
            token_start = pos - 1;
            match token {
                b'1'..=b'9' => {
                    state = LexState::ExpectDecimal;
                    next = (token - b'0') as u32;
                }
                b'0' => {
                    state = LexState::ExpectPrefix;
                    next = 0;
                }
                b'$' => {
                    state = LexState::ExpectHexFirst;
                    next = 0;
                }
                b'(' => {
                    state = LexState::Root;
                    lv.push(Token::OpenParen);
                }
                b'=' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'<' | b'>' | b'!' | b'~' => {
                    lex_operator(lv, token, &mut state);
                }
                b')' => lv.push(Token::CloseParen),
                b' ' | b'\t' => {}
                _ => {
                    // mGBA's check is `tolower(token) >= 'a' && tolower(token <= 'z')`,
                    // which accepts any character from 'a' on.
                    if token.to_ascii_lowercase() >= b'a' {
                        state = LexState::ExpectIdentifier;
                    } else {
                        state = LexState::Error;
                    }
                }
            }
        }
    }
    match state {
        LexState::ExpectBinary | LexState::ExpectDecimal | LexState::ExpectHex | LexState::ExpectPrefix => {
            lv.push(Token::Uint(next));
        }
        LexState::ExpectIdentifier => lv.push(Token::Identifier(string[token_start..pos].to_string())),
        LexState::Root | LexState::ExpectOperator2 => {}
        _ => lv.push(Token::Error),
    }
    adjusted
}

#[derive(Clone, Debug)]
struct Node {
    token: Token,
    p: Option<usize>,
    lhs: Option<usize>,
    rhs: Option<usize>,
    precedence: i32,
}

impl Node {
    fn new() -> Node {
        Node { token: Token::Error, p: None, lhs: None, rhs: None, precedence: i32::MAX }
    }
}

/// `struct ParseTree`
#[derive(Clone, Debug)]
pub struct ParseTree {
    nodes: Vec<Node>,
}

/// `_parseExpression`
fn parse_expression(nodes: &mut Vec<Node>, root: usize, lv: &mut [Token], open_parens: &mut i32) -> usize {
    let mut tree = Some(root);
    let mut pop = false;
    let mut precedence = i32::MAX;
    let mut i = 0;
    while i < lv.len() {
        let Some(t) = tree else { break };
        match lv[i].clone() {
            tok @ (Token::Identifier(_) | Token::Uint(_)) => {
                if nodes[t].token == Token::Error {
                    nodes[t].token = tok;
                } else {
                    nodes[t].token = Token::Error;
                    pop = true;
                }
                i += 1;
            }
            Token::Segment(v) => {
                let lhs = nodes.len();
                nodes.push(Node { token: Token::Uint(v), p: Some(t), lhs: None, rhs: None, precedence });
                let rhs = nodes.len();
                nodes.push(Node { token: Token::Error, p: Some(t), lhs: None, rhs: None, precedence });
                nodes[t].lhs = Some(lhs);
                nodes[t].rhs = Some(rhs);
                nodes[t].token = Token::Segment(v);
                tree = Some(rhs);
                i += 1;
            }
            Token::OpenParen => {
                *open_parens += 1;
                precedence = i32::MAX;
                i += 1;
            }
            Token::CloseParen => {
                if *open_parens <= 0 {
                    nodes[t].token = Token::Error;
                }
                *open_parens -= 1;
                i += 1;
                pop = true;
            }
            Token::Operator(mut op) => {
                if nodes[t].token == Token::Error {
                    match op {
                        Operation::Subtract => op = Operation::Negate,
                        Operation::Multiply => op = Operation::Dereference,
                        _ => {}
                    }
                    lv[i] = Token::Operator(op);
                }
                let new_precedence = op.precedence();
                if new_precedence < precedence {
                    // Push the current node down as the left-hand side.
                    let new_tree = nodes.len();
                    let mut copy = nodes[t].clone();
                    copy.p = Some(t);
                    nodes.push(copy);
                    if let Some(l) = nodes[new_tree].lhs {
                        nodes[l].p = Some(new_tree);
                    }
                    if let Some(r) = nodes[new_tree].rhs {
                        nodes[r].p = Some(new_tree);
                    }
                    nodes[t].lhs = Some(new_tree);
                    let rhs = nodes.len();
                    nodes.push(Node { token: Token::Error, p: Some(t), lhs: None, rhs: None, precedence: new_precedence });
                    nodes[t].rhs = Some(rhs);
                    precedence = new_precedence;
                    nodes[t].token = Token::Operator(op);
                    tree = Some(rhs);
                    i += 1;
                } else {
                    pop = true;
                }
            }
            Token::Error => {
                nodes[t].token = Token::Error;
                i += 1;
                pop = true;
            }
        }
        if pop {
            let t = tree.unwrap();
            if nodes[t].token == Token::Error {
                if let Some(p) = nodes[t].p {
                    nodes[p].token = Token::Error;
                }
            }
            tree = nodes[t].p;
            pop = false;
            match tree {
                None => break,
                Some(p) => precedence = nodes[p].precedence,
            }
        }
    }
    i
}

impl ParseTree {
    /// `parseTreeCreate` + `parseLexedExpression`. `None` on a syntax error.
    pub fn parse(lv: &[Token]) -> Option<ParseTree> {
        let mut nodes = vec![Node::new()];
        let mut lv = lv.to_vec();
        let mut open_parens = 0;
        parse_expression(&mut nodes, 0, &mut lv, &mut open_parens);
        if open_parens != 0 {
            nodes[0].token = Token::Error;
        }
        if nodes[0].token == Token::Error {
            None
        } else {
            Some(ParseTree { nodes })
        }
    }

    /// Lex and parse each string as one expression (the CLI's `_parseTree`).
    pub fn from_strings(strings: &[&str]) -> Option<ParseTree> {
        let mut lv = Vec::new();
        for s in strings {
            let adjusted = lex_expression(&mut lv, s, None);
            if adjusted == 0 || adjusted > s.len() {
                return None;
            }
        }
        ParseTree::parse(&lv)
    }

    /// `mDebuggerEvaluateParseTree`. `env` resolves identifiers and memory
    /// reads; returns `(value, segment)`.
    pub fn evaluate(&self, env: &mut dyn EvalEnv) -> Option<(i32, i32)> {
        let mut val = 0i32;
        let mut seg = -1i32;
        if self.eval(0, env, &mut val, &mut seg) {
            Some((val, seg))
        } else {
            None
        }
    }

    /// The traversal of `mDebuggerEvaluateParseTree`: binary operators
    /// evaluate both sides and take the left side's segment; unary operators
    /// see whatever value was last computed as their left-hand side, as in mGBA.
    fn eval(&self, n: usize, env: &mut dyn EvalEnv, val: &mut i32, seg: &mut i32) -> bool {
        let node = &self.nodes[n];
        match &node.token {
            Token::Uint(v) => {
                *val = *v as i32;
                *seg = -1;
                true
            }
            Token::Identifier(name) => match env.lookup(name) {
                Some((v, s)) => {
                    *val = v;
                    *seg = s;
                    true
                }
                None => false,
            },
            Token::Segment(_) => {
                let (Some(l), Some(r)) = (node.lhs, node.rhs) else { return false };
                if !self.eval(l, env, val, seg) {
                    return false;
                }
                let lhs = *val;
                if !self.eval(r, env, val, seg) {
                    return false;
                }
                *seg = lhs;
                true
            }
            Token::Operator(op) => {
                let binary = !matches!(op, Operation::Negate | Operation::Flip | Operation::Not | Operation::Dereference);
                if binary {
                    let (Some(l), Some(r)) = (node.lhs, node.rhs) else { return false };
                    if !self.eval(l, env, val, seg) {
                        return false;
                    }
                    let (lhs, lseg) = (*val, *seg);
                    if !self.eval(r, env, val, seg) {
                        return false;
                    }
                    let rhs = *val;
                    *seg = lseg;
                    perform_operation(env, *op, lhs, rhs, val, seg)
                } else {
                    let (lhs, lseg) = (*val, *seg);
                    let Some(r) = node.rhs else { return false };
                    if !self.eval(r, env, val, seg) {
                        return false;
                    }
                    let rhs = *val;
                    *seg = lseg;
                    perform_operation(env, *op, lhs, rhs, val, seg)
                }
            }
            _ => false,
        }
    }
}

/// What expression evaluation needs from the debugger (`mDebuggerLookupIdentifier`,
/// `busRead8`/`rawRead8`).
pub trait EvalEnv {
    fn lookup(&mut self, name: &str) -> Option<(i32, i32)>;
    fn read8(&mut self, address: u32, segment: i32) -> u32;
}

/// `_performOperation`
fn perform_operation(env: &mut dyn EvalEnv, op: Operation, mut current: i32, next: i32, value: &mut i32, segment: &mut i32) -> bool {
    use Operation::*;
    match op {
        Assign => current = next,
        Add => current = current.wrapping_add(next),
        Subtract => current = current.wrapping_sub(next),
        Multiply => current = current.wrapping_mul(next),
        Divide => {
            if next == 0 {
                return false;
            }
            current = current.wrapping_div(next);
        }
        Modulo => {
            if next == 0 {
                return false;
            }
            current = current.wrapping_rem(next);
        }
        And => current &= next,
        Or => current |= next,
        Xor => current ^= next,
        Less => current = (current < next) as i32,
        Greater => current = (current > next) as i32,
        Equal => current = (current == next) as i32,
        NotEqual => current = (current != next) as i32,
        LogicalAnd => current = (current != 0 && next != 0) as i32,
        LogicalOr => current = (current != 0 || next != 0) as i32,
        Le => current = (current <= next) as i32,
        Ge => current = (current >= next) as i32,
        Negate => current = next.wrapping_neg(),
        Flip => current = !next,
        Not => current = (next == 0) as i32,
        ShiftL => current = current.wrapping_shl(next as u32),
        ShiftR => current = current.wrapping_shr(next as u32),
        Dereference => {
            current = env.read8(next as u32, *segment) as i32;
            *segment = -1;
        }
    }
    *value = current;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Env;
    impl EvalEnv for Env {
        fn lookup(&mut self, name: &str) -> Option<(i32, i32)> {
            match name {
                "r0" => Some((5, -1)),
                "pc" => Some((0x0800_0100, -1)),
                _ => None,
            }
        }
        fn read8(&mut self, address: u32, _segment: i32) -> u32 {
            address & 0xFF
        }
    }

    fn eval(s: &str) -> Option<(i32, i32)> {
        let mut lv = Vec::new();
        lex_expression(&mut lv, s, Some(""));
        ParseTree::parse(&lv)?.evaluate(&mut Env)
    }

    #[test]
    fn evaluates_expressions() {
        assert_eq!(eval("1+2*3"), Some((7, -1)));
        assert_eq!(eval("(1+2)*3"), Some((9, -1)));
        assert_eq!(eval("0x10"), Some((16, -1)));
        assert_eq!(eval("$FF"), Some((255, -1)));
        assert_eq!(eval("0b101"), Some((5, -1)));
        assert_eq!(eval("r0 == 5"), Some((1, -1)));
        assert_eq!(eval("r0 != 5"), Some((0, -1)));
        assert_eq!(eval("r0 >= 5 && r0 < 6"), Some((1, -1)));
        assert_eq!(eval("1 << 4"), Some((16, -1)));
        assert_eq!(eval("-1"), Some((-1, -1)));
        assert_eq!(eval("*0x1234"), Some((0x34, -1)));
        assert_eq!(eval("$3:4000"), Some((0x4000, 3)));
        assert_eq!(eval("pc"), Some((0x0800_0100, -1)));
        assert_eq!(eval("nosuch"), None);
        assert_eq!(eval("1/0"), None);
        assert_eq!(eval("(1"), None);
    }
}
