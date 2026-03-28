pub mod ast;
pub mod ast_to_js;
pub mod lexer;
pub mod parser;
pub mod regexp;
mod regexp_property_data;
pub mod token;

pub use ast::*;
pub use ast_to_js::{expression_to_js, program_to_js, statement_to_js};
pub use lexer::{LexError, Lexer};
pub use parser::{ParseError, ParseOptions, Parser};
pub use regexp::*;
pub use token::{Position, Span, Token, TokenKind, TokenTag};

pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    Lexer::new(source).scan_all()
}

pub fn parse(source: &str) -> Result<Program, ParseError> {
    Parser::parse(source)
}

pub fn parse_with_options(source: &str, options: ParseOptions) -> Result<Program, ParseError> {
    Parser::parse_with_options(source, options)
}
