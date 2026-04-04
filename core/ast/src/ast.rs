use crate::{regexp::RegExpPattern, token::Span};

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Directive(Directive),
    Empty(Span),
    Debugger(Span),
    Block(BlockStatement),
    Labeled(LabeledStatement),
    ImportDeclaration(ImportDeclaration),
    ExportDeclaration(ExportDeclaration),
    VariableDeclaration(VariableDeclaration),
    FunctionDeclaration(Function),
    ClassDeclaration(Class),
    If(IfStatement),
    While(WhileStatement),
    DoWhile(DoWhileStatement),
    For(ForStatement),
    Switch(SwitchStatement),
    Return(ReturnStatement),
    Break(JumpStatement),
    Continue(JumpStatement),
    Throw(ThrowStatement),
    Try(TryStatement),
    With(WithStatement),
    Expression(ExpressionStatement),
}

impl Statement {
    pub const fn span(&self) -> Span {
        match self {
            Self::Directive(node) => node.span,
            Self::Empty(span) => *span,
            Self::Debugger(span) => *span,
            Self::Block(node) => node.span,
            Self::Labeled(node) => node.span,
            Self::ImportDeclaration(node) => node.span,
            Self::ExportDeclaration(node) => node.span(),
            Self::VariableDeclaration(node) => node.span,
            Self::FunctionDeclaration(node) => node.span,
            Self::ClassDeclaration(node) => node.span,
            Self::If(node) => node.span,
            Self::While(node) => node.span,
            Self::DoWhile(node) => node.span,
            Self::For(node) => node.span(),
            Self::Switch(node) => node.span,
            Self::Return(node) => node.span,
            Self::Break(node) => node.span,
            Self::Continue(node) => node.span,
            Self::Throw(node) => node.span,
            Self::Try(node) => node.span,
            Self::With(node) => node.span,
            Self::Expression(node) => node.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    pub value: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockStatement {
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LabeledStatement {
    pub label: Identifier,
    pub body: Box<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpressionStatement {
    pub expression: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportDeclaration {
    pub clause: Option<ImportClause>,
    pub source: StringLiteral,
    pub attributes: Option<Expression>,
    pub is_defer: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportClause {
    Default(Identifier),
    Namespace {
        default: Option<Identifier>,
        namespace: Identifier,
    },
    Named {
        default: Option<Identifier>,
        specifiers: Vec<ImportSpecifier>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportSpecifier {
    pub imported: ModuleExportName,
    pub local: Identifier,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportDeclaration {
    All(ExportAllDeclaration),
    Named(ExportNamedDeclaration),
    Default(ExportDefaultDeclaration),
    Declaration(ExportedDeclaration),
}

impl ExportDeclaration {
    pub const fn span(&self) -> Span {
        match self {
            Self::All(node) => node.span,
            Self::Named(node) => node.span,
            Self::Default(node) => node.span,
            Self::Declaration(node) => node.span(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportAllDeclaration {
    pub exported: Option<ModuleExportName>,
    pub source: StringLiteral,
    pub attributes: Option<Expression>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportNamedDeclaration {
    pub specifiers: Vec<ExportSpecifier>,
    pub source: Option<StringLiteral>,
    pub attributes: Option<Expression>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportDefaultDeclaration {
    pub declaration: ExportDefaultKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportDefaultKind {
    Function(Function),
    Class(Class),
    Expression(Expression),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportedDeclaration {
    Variable(VariableDeclaration),
    Function(Function),
    Class(Class),
}

impl ExportedDeclaration {
    pub const fn span(&self) -> Span {
        match self {
            Self::Variable(node) => node.span,
            Self::Function(node) => node.span,
            Self::Class(node) => node.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportSpecifier {
    pub local: ModuleExportName,
    pub exported: ModuleExportName,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModuleExportName {
    Identifier(Identifier),
    String(StringLiteral),
}

impl ModuleExportName {
    pub const fn span(&self) -> Span {
        match self {
            Self::Identifier(node) => node.span,
            Self::String(node) => node.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariableDeclaration {
    pub kind: VariableKind,
    pub declarations: Vec<VariableDeclarator>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableKind {
    Var,
    Let,
    Const,
    Using,
    AwaitUsing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariableDeclarator {
    pub pattern: Pattern,
    pub init: Option<Expression>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub id: Option<Identifier>,
    pub params: Vec<Pattern>,
    pub body: BlockStatement,
    pub is_async: bool,
    pub is_generator: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub decorators: Vec<Expression>,
    pub id: Option<Identifier>,
    pub super_class: Option<Expression>,
    pub body: Vec<ClassElement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassElement {
    Empty(Span),
    StaticBlock(BlockStatement),
    Method(ClassMethod),
    Field(ClassField),
}

impl ClassElement {
    pub const fn span(&self) -> Span {
        match self {
            Self::Empty(span) => *span,
            Self::StaticBlock(block) => block.span,
            Self::Method(method) => method.span,
            Self::Field(field) => field.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassMethod {
    pub decorators: Vec<Expression>,
    pub key: PropertyKey,
    pub value: Function,
    pub kind: MethodKind,
    pub is_static: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassField {
    pub decorators: Vec<Expression>,
    pub key: PropertyKey,
    pub value: Option<Expression>,
    pub is_static: bool,
    pub is_accessor: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodKind {
    Method,
    Getter,
    Setter,
    Constructor,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStatement {
    pub test: Expression,
    pub consequent: Box<Statement>,
    pub alternate: Option<Box<Statement>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhileStatement {
    pub test: Expression,
    pub body: Box<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DoWhileStatement {
    pub body: Box<Statement>,
    pub test: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForStatement {
    Classic(ForClassicStatement),
    In(ForEachStatement),
    Of(ForEachStatement),
}

impl ForStatement {
    pub const fn span(&self) -> Span {
        match self {
            Self::Classic(node) => node.span,
            Self::In(node) | Self::Of(node) => node.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForClassicStatement {
    pub init: Option<ForInit>,
    pub test: Option<Expression>,
    pub update: Option<Expression>,
    pub body: Box<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForInit {
    VariableDeclaration(VariableDeclaration),
    Expression(Expression),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForEachStatement {
    pub left: ForLeft,
    pub right: Expression,
    pub is_await: bool,
    pub body: Box<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForLeft {
    VariableDeclaration(VariableDeclaration),
    Pattern(Pattern),
    Expression(Expression),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchStatement {
    pub discriminant: Expression,
    pub cases: Vec<SwitchCase>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SwitchCase {
    pub test: Option<Expression>,
    pub consequent: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStatement {
    pub argument: Option<Expression>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JumpStatement {
    pub label: Option<Identifier>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThrowStatement {
    pub argument: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TryStatement {
    pub block: BlockStatement,
    pub handler: Option<CatchClause>,
    pub finalizer: Option<BlockStatement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatchClause {
    pub param: Option<Pattern>,
    pub body: BlockStatement,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WithStatement {
    pub object: Expression,
    pub body: Box<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Identifier {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Identifier(Identifier),
    Array(ArrayPattern),
    Object(ObjectPattern),
    Rest(RestPattern),
    Assignment(AssignmentPattern),
}

impl Pattern {
    pub const fn span(&self) -> Span {
        match self {
            Self::Identifier(node) => node.span,
            Self::Array(node) => node.span,
            Self::Object(node) => node.span,
            Self::Rest(node) => node.span,
            Self::Assignment(node) => node.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayPattern {
    pub elements: Vec<Option<Pattern>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectPattern {
    pub properties: Vec<ObjectPatternProperty>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectPatternProperty {
    Property {
        key: PropertyKey,
        value: Box<Pattern>,
        shorthand: bool,
        span: Span,
    },
    Rest {
        argument: Box<Pattern>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestPattern {
    pub argument: Box<Pattern>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentPattern {
    pub left: Box<Pattern>,
    pub right: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Identifier(Identifier),
    PrivateIdentifier(Identifier),
    Literal(Literal),
    This(Span),
    Super(Span),
    Array(ArrayExpression),
    Object(ObjectExpression),
    Function(Box<Function>),
    ArrowFunction(Box<ArrowFunction>),
    Class(Box<Class>),
    TaggedTemplate(Box<TaggedTemplateExpression>),
    MetaProperty(Box<MetaPropertyExpression>),
    Yield(Box<YieldExpression>),
    Await(Box<AwaitExpression>),
    Unary(Box<UnaryExpression>),
    Update(Box<UpdateExpression>),
    Binary(Box<BinaryExpression>),
    Logical(Box<LogicalExpression>),
    Assignment(Box<AssignmentExpression>),
    Conditional(Box<ConditionalExpression>),
    Sequence(SequenceExpression),
    Call(Box<CallExpression>),
    Member(Box<MemberExpression>),
    New(Box<NewExpression>),
}

impl Expression {
    pub const fn span(&self) -> Span {
        match self {
            Self::Identifier(node) => node.span,
            Self::PrivateIdentifier(node) => node.span,
            Self::Literal(node) => node.span(),
            Self::This(span) => *span,
            Self::Super(span) => *span,
            Self::Array(node) => node.span,
            Self::Object(node) => node.span,
            Self::Function(node) => node.span,
            Self::ArrowFunction(node) => node.span,
            Self::Class(node) => node.span,
            Self::TaggedTemplate(node) => node.span,
            Self::MetaProperty(node) => node.span,
            Self::Yield(node) => node.span,
            Self::Await(node) => node.span,
            Self::Unary(node) => node.span,
            Self::Update(node) => node.span,
            Self::Binary(node) => node.span,
            Self::Logical(node) => node.span,
            Self::Assignment(node) => node.span,
            Self::Conditional(node) => node.span,
            Self::Sequence(node) => node.span,
            Self::Call(node) => node.span,
            Self::Member(node) => node.span,
            Self::New(node) => node.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrowFunction {
    pub params: Vec<Pattern>,
    pub body: ArrowBody,
    pub is_async: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArrowBody {
    Expression(Box<Expression>),
    Block(BlockStatement),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaggedTemplateExpression {
    pub tag: Expression,
    pub quasi: TemplateLiteral,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetaPropertyExpression {
    pub meta: Identifier,
    pub property: Identifier,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct YieldExpression {
    pub argument: Option<Expression>,
    pub delegate: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AwaitExpression {
    pub argument: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null(Span),
    Boolean(BooleanLiteral),
    Number(NumberLiteral),
    String(StringLiteral),
    Template(TemplateLiteral),
    RegExp(RegExpLiteral),
}

impl Literal {
    pub const fn span(&self) -> Span {
        match self {
            Self::Null(span) => *span,
            Self::Boolean(node) => node.span,
            Self::Number(node) => node.span,
            Self::String(node) => node.span,
            Self::Template(node) => node.span,
            Self::RegExp(node) => node.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BooleanLiteral {
    pub value: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NumberLiteral {
    pub raw: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StringLiteral {
    pub value: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateLiteral {
    pub value: String,
    pub raw: String,
    pub invalid_escape: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegExpLiteral {
    pub body: String,
    pub flags: String,
    pub pattern: RegExpPattern,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayExpression {
    pub elements: Vec<Option<ArrayElement>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArrayElement {
    Expression(Expression),
    Spread { argument: Expression, span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectExpression {
    pub properties: Vec<ObjectProperty>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectProperty {
    Property {
        key: PropertyKey,
        value: Expression,
        shorthand: bool,
        kind: ObjectPropertyKind,
        span: Span,
    },
    Spread {
        argument: Expression,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectPropertyKind {
    Init,
    Method,
    Getter,
    Setter,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PropertyKey {
    Identifier(Identifier),
    PrivateName(Identifier),
    String(StringLiteral),
    Number(NumberLiteral),
    Computed {
        expression: Box<Expression>,
        span: Span,
    },
}

impl PropertyKey {
    pub const fn span(&self) -> Span {
        match self {
            Self::Identifier(node) => node.span,
            Self::PrivateName(node) => node.span,
            Self::String(node) => node.span,
            Self::Number(node) => node.span,
            Self::Computed { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnaryExpression {
    pub operator: UnaryOperator,
    pub argument: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    Delete,
    Void,
    Typeof,
    Positive,
    Negative,
    LogicalNot,
    BitNot,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateExpression {
    pub operator: UpdateOperator,
    pub argument: Expression,
    pub prefix: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOperator {
    Increment,
    Decrement,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinaryExpression {
    pub operator: BinaryOperator,
    pub left: Expression,
    pub right: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Exponentiate,
    LeftShift,
    SignedRightShift,
    UnsignedRightShift,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    Equality,
    StrictEquality,
    Inequality,
    StrictInequality,
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    In,
    PrivateIn,
    Instanceof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogicalExpression {
    pub operator: LogicalOperator,
    pub left: Expression,
    pub right: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOperator {
    And,
    Or,
    NullishCoalescing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentExpression {
    pub operator: AssignmentOperator,
    pub left: Expression,
    pub right: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentOperator {
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    PowAssign,
    ShlAssign,
    SarAssign,
    ShrAssign,
    AndAssign,
    XorAssign,
    OrAssign,
    LogicalAndAssign,
    LogicalOrAssign,
    NullishAssign,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConditionalExpression {
    pub test: Expression,
    pub consequent: Expression,
    pub alternate: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SequenceExpression {
    pub expressions: Vec<Expression>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallExpression {
    pub callee: Expression,
    pub arguments: Vec<CallArgument>,
    pub optional: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallArgument {
    Expression(Expression),
    Spread { argument: Expression, span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberExpression {
    pub object: Expression,
    pub property: MemberProperty,
    pub optional: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemberProperty {
    Identifier(Identifier),
    PrivateName(Identifier),
    Computed {
        expression: Box<Expression>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewExpression {
    pub callee: Expression,
    pub arguments: Vec<CallArgument>,
    pub span: Span,
}
