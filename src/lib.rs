#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]
#![cfg_attr(test, feature(test))]
#![feature(splice, slice_patterns)]

extern crate petgraph;
#[cfg(test)]
extern crate test;

pub mod parse;
#[macro_use]
pub mod evaluator;
mod specialization;

use std::mem::{transmute, transmute_copy};
use std::fmt;
use std::iter::repeat;
use std::rc::Rc;
use std::cell::{Cell, UnsafeCell};
use std::hash::{Hash, Hasher};
use std::collections::HashMap;
use evaluator::{compile_finalized_expr, Instr, State};

type EvaluationResult<T> = Result<T, EvaluationError>;

#[derive(Debug)]
pub struct InnerCustomFunc {
    arg_count: usize,
    body: FinalizedExpr,
    byte_code: UnsafeCell<Vec<Instr>>,
}

#[derive(Debug, Clone)]
pub struct CustomFunc(Rc<InnerCustomFunc>);

impl Hash for CustomFunc {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let ptr = unsafe { transmute_copy::<Rc<_>, usize>(&self.0) };
        state.write_usize(ptr);
    }
}

impl PartialEq for CustomFunc {
    fn eq(&self, other: &CustomFunc) -> bool {
        unsafe {
            transmute_copy::<Rc<_>, usize>(&self.0) == transmute_copy::<Rc<_>, usize>(&other.0)
        }
    }
}

impl Eq for CustomFunc {}

impl CustomFunc {
    pub fn compile<'s>(&'s self, state: &State) -> EvaluationResult<&'s [Instr]> {
        unsafe {
            let borrowed = self.0.byte_code.get().as_ref().unwrap();
            if !borrowed.is_empty() {
                Ok(transmute(&borrowed[..]))
            } else {
                let mut_borrowed = self.0.byte_code.get().as_mut().unwrap();
                *mut_borrowed = compile_finalized_expr(self.0.body.clone(), state)?;
                Ok(transmute(&mut_borrowed[..]))
            }
        }
    }

    pub fn from_byte_code(arg_count: usize, bytecode: Vec<Instr>) -> Self {
        CustomFunc(Rc::new(InnerCustomFunc {
            arg_count: arg_count,
            // dummy
            body: FinalizedExpr::Value(LispValue::Boolean(false)),
            byte_code: UnsafeCell::new(bytecode),
        }))
    }

    pub fn pretty_print(&self, indent: usize) -> String {
        let mut result = String::new();

        for i in 0..self.0.arg_count {
            if i > 0 {
                result.push(' ');
            }
            result.push_str(&format!("${}", i));
        }

        result.push_str(&format!(" ->\n{}", indent_to_string(indent + 1)));
        result + &self.0.body.pretty_print(indent + 1)
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub enum BuiltIn {
    AddOne,
    SubOne,
    Cons,
    Cdr,
    Car,
    List,
    CheckZero,
    CheckNull,
    CheckType(ArgType),
}

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub enum ArgType {
    Integer,
    Boolean,
    Function,
    List,
}

impl BuiltIn {
    fn from_str(s: &str) -> Option<BuiltIn> {
        match s {
            "add1" => Some(BuiltIn::AddOne),
            "sub1" => Some(BuiltIn::SubOne),
            "cons" => Some(BuiltIn::Cons),
            "cdr" => Some(BuiltIn::Cdr),
            "car" => Some(BuiltIn::Car),
            "list" => Some(BuiltIn::List),
            "zero?" => Some(BuiltIn::CheckZero),
            "null?" => Some(BuiltIn::CheckNull),
            "int?" => Some(BuiltIn::CheckType(ArgType::Integer)),
            "bool?" => Some(BuiltIn::CheckType(ArgType::Boolean)),
            "list?" => Some(BuiltIn::CheckType(ArgType::List)),
            "fun?" => Some(BuiltIn::CheckType(ArgType::Function)),
            _ => None,
        }
    }
}

impl fmt::Display for BuiltIn {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: is there a way to ensure this is consistent with the
        // from_str function?
        let str = match *self {
            BuiltIn::AddOne => "add1",
            BuiltIn::SubOne => "sub1",
            BuiltIn::Cons => "cons",
            BuiltIn::Cdr => "cdr",
            BuiltIn::Car => "car",
            BuiltIn::List => "list",
            BuiltIn::CheckZero => "zero?",
            BuiltIn::CheckNull => "null?",
            BuiltIn::CheckType(ArgType::Function) => "fun?",
            BuiltIn::CheckType(ArgType::Boolean) => "bool?",
            BuiltIn::CheckType(ArgType::Integer) => "int?",
            BuiltIn::CheckType(ArgType::List) => "list?",
        };

        write!(f, "{}", str)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LispFunc {
    BuiltIn(BuiltIn),
    Custom(CustomFunc),
}

impl LispFunc {
    pub fn new_custom(arg_count: usize, body: FinalizedExpr) -> LispFunc {
        LispFunc::Custom(CustomFunc(Rc::new(InnerCustomFunc {
            arg_count: arg_count,
            body: body,
            byte_code: UnsafeCell::new(Vec::new()),
        })))
    }

    pub fn create_continuation(
        f: CustomFunc,
        total_args: usize,
        supplied_args: usize,
        stack: &[LispValue],
    ) -> LispFunc {
        let arg_count = total_args - supplied_args;
        let funk = Box::new(FinalizedExpr::Value(
            LispValue::Function(LispFunc::Custom(f)),
        ));
        let mut arg_vec: Vec<_> = stack[..supplied_args]
            .iter()
            .cloned()
            .map(FinalizedExpr::Value)
            .collect();
        arg_vec.extend(
            // TODO: check that we can get away with just setting scope to 0
            // or whether we need to be more clever
            (0..total_args - supplied_args).map(|o| FinalizedExpr::Argument(o, 0, true)),
        );

        Self::new_custom(
            arg_count,
            FinalizedExpr::FunctionCall(funk, arg_vec, true, false),
        )
    }

    pub fn pretty_print(&self, indent: usize) -> String {
        match *self {
            LispFunc::BuiltIn(name) => format!("{:?}", name),
            LispFunc::Custom(ref c) => c.pretty_print(indent),
        }
    }
}

fn indent_to_string(indent: usize) -> String {
    repeat(' ').take(indent * 4).collect()
}

impl fmt::Display for LispFunc {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.pretty_print(0))
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LispMacro {
    Define,
    Cond,
    Lambda,
}

impl LispMacro {
    fn from_str(s: &str) -> Option<LispMacro> {
        match s {
            "define" => Some(LispMacro::Define),
            "cond" => Some(LispMacro::Cond),
            "lambda" => Some(LispMacro::Lambda),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TopExpr {
    Define(String, LispExpr),
    Regular(FinalizedExpr),
}

// TODO: replace bools by two variant enums
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum FinalizedExpr {
    // Arg count, scope level, body
    Lambda(usize, usize, Box<FinalizedExpr>),
    // test expr, true branch, false branch
    Cond(Box<(FinalizedExpr, FinalizedExpr, FinalizedExpr)>),
    Variable(String),
    Value(LispValue),
    // Offset from stack pointer, scope level, moveable
    Argument(usize, usize, bool),
    // function, arguments, tail-call, self-call
    FunctionCall(Box<FinalizedExpr>, Vec<FinalizedExpr>, bool, bool),
}

impl fmt::Display for FinalizedExpr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.pretty_print(0))
    }
}

impl FinalizedExpr {
    // Resolves references to function arguments. Used when creating closures.
    pub fn replace_args(&self, scope_level: usize, stack: &[LispValue]) -> FinalizedExpr {
        match *self {
            FinalizedExpr::Argument(index, arg_scope, _is_move) if arg_scope < scope_level => {
                // TODO: we could actually do a move here!
                FinalizedExpr::Value(stack[index].clone())
            }
            FinalizedExpr::FunctionCall(ref head, ref vec, is_tail_call, is_self_call) => {
                FinalizedExpr::FunctionCall(
                    Box::new(head.replace_args(scope_level, stack)),
                    vec.iter()
                        .map(|e| e.replace_args(scope_level, stack))
                        .collect(),
                    is_tail_call,
                    is_self_call,
                )
            }
            FinalizedExpr::Cond(ref triple) => {
                let (ref test, ref true_expr, ref false_expr) = **triple;
                FinalizedExpr::Cond(Box::new((
                    test.replace_args(scope_level, stack),
                    true_expr.replace_args(scope_level, stack),
                    false_expr.replace_args(scope_level, stack),
                )))
            }
            FinalizedExpr::Lambda(arg_c, scope, ref body) => FinalizedExpr::Lambda(
                arg_c,
                scope,
                Box::new(body.replace_args(scope_level, stack)),
            ),
            ref x => x.clone(),
        }
    }

    pub fn pretty_print(&self, indent: usize) -> String {
        match *self {
            FinalizedExpr::Argument(offset, scope, is_move) => {
                format!("{}$({}, {})", if is_move { "m" } else { "" }, offset, scope)
            }
            FinalizedExpr::Value(ref v) => v.pretty_print(indent),
            FinalizedExpr::Variable(ref name) => name.clone(),
            FinalizedExpr::Cond(ref triple) => {
                let (ref test, ref true_expr, ref false_expr) = **triple;
                let expr_iter = Some(&*true_expr).into_iter().chain(Some(&*false_expr));
                format_list(
                    indent,
                    "cond".to_owned(),
                    &test.pretty_print(indent),
                    expr_iter,
                )
            }
            FinalizedExpr::Lambda(arg_c, scope, ref body) => format!(
                "lambda ({}, {}) -> {}",
                arg_c,
                scope,
                body.pretty_print(indent)
            ),
            FinalizedExpr::FunctionCall(ref funk, ref args, is_tail_call, is_self_call) => {
                let prefix = match (is_self_call, is_tail_call) {
                    (true, true) => "r".to_owned(),
                    (false, true) => "t".to_owned(),
                    (_, _) => String::new(),
                };

                format_list(indent, prefix, &funk.pretty_print(indent), args.iter())
            }
        }
    }
}

fn format_list<'a, I: Iterator<Item = &'a FinalizedExpr>>(
    indent: usize,
    prefix: String,
    first_item: &str,
    expr_list: I,
) -> String {
    let mut result = prefix;

    result.push('{');
    result.push_str(first_item);

    for expr in expr_list {
        result.push('\n');
        result.push_str(&indent_to_string(indent));
        result.push_str(&expr.pretty_print(indent));
    }

    result.push('}');
    result
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LispExpr {
    Macro(LispMacro),
    Value(LispValue),
    OpVar(String),
    Call(Vec<LispExpr>),
}

impl LispExpr {
    pub fn into_top_expr(self, state: &State) -> EvaluationResult<TopExpr> {
        let is_define = if let &LispExpr::Call(ref expr_list) = &self {
            Some(&LispExpr::Macro(LispMacro::Define)) == expr_list.get(0)
        } else {
            false
        };

        // This feels kind of clumsy
        if is_define {
            match self {
                LispExpr::Call(expr_list) => {
                    let mut call_iter = expr_list.into_iter();
                    destructure!(call_iter, [mac, opvar, definition], {
                        if let LispExpr::OpVar(n) = opvar {
                            Ok(TopExpr::Define(n, definition))
                        } else {
                            Err(EvaluationError::BadDefine)
                        }
                    })
                }
                _ => unreachable!(),
            }
        } else {
            Ok(TopExpr::Regular(
                self.finalize(0, &HashMap::new(), state, true, None)?,
            ))
        }
    }

    // TODO: cleanup arguments. maybe pass around a context?
    pub fn finalize(
        self,
        scope_level: usize,
        // maps symbols to (scope_level, offset, moveable)
        arguments: &HashMap<String, (usize, usize, Cell<bool>)>,
        state: &State,
        can_tail_call: bool,
        own_name: Option<&str>,
    ) -> EvaluationResult<FinalizedExpr> {
        Ok(match self {
            LispExpr::Value(v) => FinalizedExpr::Value(v),
            LispExpr::OpVar(n) => {
                // So if we encounter a symbol, it could be two things:
                // a function argument, in which case it should be in the arguments map
                // a reference to something in our state.
                // Function arguments take precendence.
                if let Some(&(arg_scope, arg_offset, ref moveable)) = arguments.get(&n) {
                    let is_moveable = moveable.replace(false);
                    FinalizedExpr::Argument(arg_offset, arg_scope, is_moveable)
                } else {
                    FinalizedExpr::Variable(n)
                }
            }
            LispExpr::Macro(..) => {
                return Err(EvaluationError::UnexpectedOperator);
            }
            LispExpr::Call(expr_list) => {
                // TODO: add test for empty calls
                let mut expr_iter = expr_list.into_iter();
                let head_expr = match expr_iter.next() {
                    Some(head) => head,
                    None => return Err(EvaluationError::EmptyListEvaluation),
                };

                match head_expr {
                    LispExpr::Macro(LispMacro::Cond) => {
                        destructure!(expr_iter, [test_expr, true_expr, false_expr], {
                            let false_expr_args = arguments.clone();
                            let finalized_false_expr = false_expr.finalize(
                                scope_level,
                                &false_expr_args,
                                state,
                                can_tail_call,
                                own_name,
                            )?;
                            let finalized_true_expr = true_expr.finalize(
                                scope_level,
                                arguments,
                                state,
                                can_tail_call,
                                own_name,
                            )?;

                            for key in arguments.keys() {
                                let new_value =
                                    arguments[key].2.get() && false_expr_args[key].2.get();
                                arguments[key].2.replace(new_value);
                            }

                            FinalizedExpr::Cond(Box::new((
                                test_expr.finalize(scope_level, arguments, state, false, own_name)?,
                                finalized_true_expr,
                                finalized_false_expr,
                            )))
                        })
                    }
                    LispExpr::Macro(LispMacro::Lambda) => {
                        destructure!(expr_iter, [arg_list, body], {
                            if let LispExpr::Call(ref arg_vec) = arg_list {
                                // Add arguments to the arguments map, overwriting existing
                                // ones if they have the same symbol.
                                // FIXME: movement of arguments that aren't overwritten are screwed by this
                                let mut new_arguments = arguments.clone();
                                let num_args = arg_vec.len();

                                for (offset, expr) in arg_vec.into_iter().enumerate() {
                                    let symbol = match *expr {
                                        LispExpr::OpVar(ref name) => Ok(&name[..]),
                                        _ => Err(EvaluationError::MalformedDefinition),
                                    }?;

                                    new_arguments.insert(
                                        symbol.to_owned(),
                                        (scope_level, offset, Cell::new(true)),
                                    );
                                }

                                FinalizedExpr::Lambda(
                                    num_args,
                                    scope_level,
                                    Box::new(body.finalize(
                                        scope_level + 1,
                                        &new_arguments,
                                        state,
                                        true,
                                        own_name,
                                    )?),
                                )
                            } else {
                                return Err(EvaluationError::ArgumentTypeMismatch);
                            }
                        })
                    }
                    // Defines should be caught by into_top_expr
                    LispExpr::Macro(LispMacro::Define) => {
                        return Err(EvaluationError::MalformedDefinition)
                    }
                    // Function evaluation
                    _ => {
                        let is_self_call = if let LispExpr::OpVar(ref n) = head_expr {
                            if let Some(self_name) = own_name {
                                n == self_name
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        // We traverse the arguments from last to first to make sure
                        // we get the argument moves correctly. The last arguments
                        // get to use the moves first.
                        let mut arg_finalized_expr = Vec::new();
                        for e in expr_iter.rev() {
                            let finalized =
                                e.finalize(scope_level, arguments, state, false, own_name)?;
                            arg_finalized_expr.push(finalized);
                        }
                        arg_finalized_expr.reverse();

                        let funk =
                            head_expr.finalize(scope_level, arguments, state, false, own_name)?;
                        FinalizedExpr::FunctionCall(
                            Box::new(funk),
                            arg_finalized_expr,
                            can_tail_call,
                            is_self_call,
                        )
                    }
                }
            }
        })
    }

    pub fn pretty_print(&self, indent: usize) -> String {
        match *self {
            LispExpr::Value(ref v) => v.pretty_print(indent),
            LispExpr::OpVar(ref name) => name.clone(),
            LispExpr::Macro(ref mac) => format!("{:?}", mac),
            LispExpr::Call(ref expr_vec) => {
                let mut result = String::new();

                result.push('{');

                for (idx, expr) in expr_vec.iter().enumerate() {
                    if idx > 0 {
                        result.push('\n');
                        result.push_str(&indent_to_string(indent));
                    }

                    result.push_str(&expr.pretty_print(indent));
                }

                result.push('}');
                result
            }
        }
    }
}

impl fmt::Display for LispExpr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.pretty_print(0))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum EvaluationError {
    UnexpectedOperator,
    ArgumentCountMismatch,
    ArgumentTypeMismatch,
    EmptyListEvaluation,
    NonFunctionApplication,
    SubZero,
    EmptyList,
    UnknownVariable(String),
    MalformedDefinition,
    BadDefine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LispValue {
    Boolean(bool),
    Integer(u64),
    Function(LispFunc),
    List(Vec<LispValue>),
}

impl LispValue {
    pub fn get_type(&self) -> ArgType {
        match *self {
            LispValue::Boolean(..) => ArgType::Boolean,
            LispValue::Integer(..) => ArgType::Integer,
            LispValue::Function(..) => ArgType::Function,
            LispValue::List(..) => ArgType::List,
        }
    }

    pub fn pretty_print(&self, indent: usize) -> String {
        match *self {
            LispValue::Function(ref func) => format!("[{}]", func.pretty_print(indent)),
            LispValue::Integer(i) => i.to_string(),
            LispValue::Boolean(true) => "#t".into(),
            LispValue::Boolean(false) => "#f".into(),
            LispValue::List(ref vec) => {
                let mut result = "(".to_string();

                for (idx, val) in vec.iter().enumerate() {
                    if idx > 0 {
                        result.push(' ');
                    }

                    result.push_str(&val.pretty_print(indent));
                }

                result.push(')');
                result
            }
        }
    }
}

impl fmt::Display for LispValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.pretty_print(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::parse::{parse_lisp_string, ParseError};
    use super::evaluator::State;
    use std::convert::From;

    #[derive(Debug, PartialEq, Eq)]
    enum LispError {
        Parse(ParseError),
        Evaluation(EvaluationError),
    }

    impl From<EvaluationError> for LispError {
        fn from(err: EvaluationError) -> LispError {
            LispError::Evaluation(err)
        }
    }

    impl From<ParseError> for LispError {
        fn from(err: ParseError) -> LispError {
            LispError::Parse(err)
        }
    }

    fn check_lisp<'i, I>(commands: I) -> Result<LispValue, LispError>
    where
        I: IntoIterator<Item = &'i str>,
    {
        let mut state = State::default();
        let mut last_ret_val = None;

        for cmd in commands {
            let expr = parse_lisp_string(cmd)?;
            last_ret_val = Some(evaluator::eval(expr, &mut state)?);
        }

        Ok(last_ret_val.unwrap())
    }

    fn check_lisp_ok<'i, I>(commands: I, expected_out: &str)
    where
        I: IntoIterator<Item = &'i str>,
    {
        assert_eq!(expected_out, check_lisp(commands).unwrap().to_string());
    }

    fn check_lisp_err<'i, I>(commands: I, expected_err: LispError)
    where
        I: IntoIterator<Item = &'i str>,
    {
        assert_eq!(expected_err, check_lisp(commands).unwrap_err());
    }

    #[test]
    fn add_bytecode() {
        let add = check_lisp(vec![
            "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
            "(add 0 0)",
            "(car (list add))",
        ]).unwrap();

        match add {
            LispValue::Function(LispFunc::Custom(f)) => {
                assert_eq!(
                    vec![
                        Instr::MoveArgument(0),
                        Instr::Jump(1),
                        Instr::Recurse(2),
                        Instr::SubOne,
                        Instr::MoveArgument(1),
                        Instr::AddOne,
                        Instr::MoveArgument(0),
                        Instr::CondJump(6),
                        Instr::CheckZero,
                        Instr::CloneArgument(1),
                    ],
                    unsafe { f.0.byte_code.get().as_ref().unwrap().clone() }
                );
            }
            _ => panic!("expected function!"),
        }
    }

    #[test]
    fn shadowing() {
        check_lisp_ok(
            vec![
                "(define f (lambda (x) (lambda (x) x)))",
                "(list ((f #t) 3) ((f (list)) #f))",
            ],
            "(3 #f)",
        );
    }

    #[test]
    fn shadowing_two() {
        check_lisp_ok(
            vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "((lambda (x) (add ((lambda (x) (add1 x)) x) x)) 10)",
            ],
            "21",
        );
    }

    #[test]
    fn shadowing_three() {
        check_lisp_ok(
            vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "((lambda (x) (add x ((lambda (x) (add (sub1 x) x)) x))) 4)",
            ],
            "11",
        );
    }

    #[test]
    fn display_int_val() {
        let val = LispValue::Integer(5);
        assert_eq!("5", val.to_string());
    }

    #[test]
    fn display_list_val() {
        let val = LispValue::List(vec![LispValue::Integer(1), LispValue::List(vec![])]);
        assert_eq!("(1 ())", val.to_string());
    }

    #[test]
    fn function_add() {
        check_lisp_ok(
            vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "(add 77 12)",
            ],
            "89",
        );
    }

    #[test]
    fn function_multiply() {
        check_lisp_ok(
            vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "(define mult (lambda (x y) (cond (zero? y) 0 (add x (mult x (sub1 y))))))",
                "(mult 7 3)",
            ],
            "21",
        );
    }

    #[test]
    fn map2_zip() {
        check_lisp_ok(
            vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "(define or (lambda (x y) (cond x #t y)))",
                "(define zip (lambda (x y) (cond (or (null? x) (null? y)) (list) (cons (list (car x) (car y)) (zip (cdr x) (cdr y))))))",
                "(define map2 (lambda (f l) (cond (null? l) (list) (cons (f (car (cdr (car l))) (car (car l))) (map2 f (cdr l))))))",
                "(map2 add (zip (list 1 2 3 4 5) (list 0 20 40 60)))",
            ],
            "(2 23 44 65)",
        );
    }

    #[test]
    fn function_def() {
        check_lisp_ok(
            vec!["(define add2 (lambda (x) (add1 (add1 x))))", "(add2 5)"],
            "7",
        );
    }

    #[test]
    fn variable_overwrite() {
        check_lisp_err(
            vec!["(define x 1)", "(define x 1000)", "(add1 x)"],
            LispError::Evaluation(EvaluationError::BadDefine),
        );
    }

    #[test]
    fn check_int() {
        check_lisp_ok(
            vec![
                "(define map (lambda (f xs) (cond (null? xs) (list) (cons (f (car xs)) (map f (cdr xs))))))",
                "(map int? (list 1 2 #t (list 3 4) add1 0))",
            ],
            "(#t #t #f #f #f #t)",
        );
    }

    #[test]
    fn check_bool() {
        check_lisp_ok(
            vec![
                "(list (bool? #t) (bool? #f) (bool? 5) (bool? (list #t)) (bool? bool?))",
            ],
            "(#t #t #f #f #f)",
        );
    }

    #[test]
    fn check_fun() {
        check_lisp_ok(
            vec![
                "(list (fun? #t) (fun? fun?) (fun? 0) (fun? bool?) (fun? (lambda (x) #t)) (fun? #t))",
            ],
            "(#f #t #f #t #t #f)",
        );
    }

    #[test]
    fn check_list() {
        check_lisp_ok(
            vec!["(list (list? list) (list? list?) (list? (list)))"],
            "(#f #f #t)",
        );
    }

    #[test]
    fn is_null_empty_list() {
        check_lisp_ok(vec!["(null? (list))"], "#t");
    }

    #[test]
    fn cdr() {
        check_lisp_ok(vec!["(cdr (list 1 2 3 4))"], "(1 2 3)");
    }

    #[test]
    fn is_zero_of_zero() {
        check_lisp_ok(vec!["(zero? 0)"], "#t");
    }

    #[test]
    fn is_zero_of_nonzero() {
        check_lisp_ok(vec!["(zero? 5)"], "#f");
    }

    #[test]
    fn is_zero_of_list() {
        check_lisp_err(
            vec!["(zero? (list 0))"],
            LispError::Evaluation(EvaluationError::ArgumentTypeMismatch),
        );
    }

    #[test]
    fn is_zero_two_args() {
        check_lisp_err(
            vec!["(zero? 0 0)"],
            LispError::Evaluation(EvaluationError::ArgumentCountMismatch),
        );
    }

    #[test]
    fn too_few_arguments() {
        check_lisp_err(
            vec!["(add1)"],
            LispError::Evaluation(EvaluationError::ArgumentCountMismatch),
        );
    }

    #[test]
    fn lower_level_define() {
        check_lisp_err(
            vec!["(list (define x 5))"],
            LispError::Evaluation(EvaluationError::MalformedDefinition),
        );
    }

    #[test]
    fn too_many_arguments() {
        check_lisp_err(
            vec!["(lambda f (x) (add1 x) ())"],
            LispError::Evaluation(EvaluationError::ArgumentCountMismatch),
        );
    }

    #[test]
    fn non_function_app() {
        check_lisp_err(
            vec!["(10 3)"],
            LispError::Evaluation(EvaluationError::NonFunctionApplication),
        );
    }

    #[test]
    fn unexpected_operator() {
        check_lisp_err(
            vec!["(cond cond cond cond)"],
            LispError::Evaluation(EvaluationError::UnexpectedOperator),
        );
    }

    #[test]
    fn undefined_function() {
        check_lisp_err(
            vec!["(first (list 10 3))"],
            LispError::Evaluation(EvaluationError::UnknownVariable("first".into())),
        );
    }

    #[test]
    fn test_variable_list() {
        check_lisp_ok(
            vec![
                "(define x 3)",
                "(define + (lambda (x y) (cond (zero? y) x (+ (add1 x) (sub1 y)))))",
                "(list x 1 (+ 1 x) 5)",
            ],
            "(3 1 4 5)",
        );
    }

    #[test]
    fn eval_empty_list() {
        check_lisp_ok(vec!["(list)"], "()");
    }

    #[test]
    fn map() {
        check_lisp_ok(
            vec![
                "(define map (lambda (f xs) (cond (null? xs) (list) (cons (f (car xs)) (map f (cdr xs))))))",
                "(map add1 (list 1 2 3))",
            ],
            "(2 3 4)",
        );
    }

    #[test]
    fn lambda() {
        check_lisp_ok(
            vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "(define mult (lambda (x y) (cond (zero? y) 0 (add x (mult x (sub1 y))))))",
                "(define map (lambda (f xs) (cond (null? xs) (list) (cons (f (car xs)) (map f (cdr xs))))))",
                "(map (lambda (x) (mult x x)) (list 1 2 3))",
            ],
            "(1 4 9)",
        );
    }

    const SORT_COMMANDS: &[&str] = &[
        "(define filter (lambda (f xs) (cond (null? xs) (list) (cond (f (car xs)) (cons (car xs) (filter f (cdr xs))) (filter f (cdr xs))))))",
        "(define not (lambda (t) (cond t #f #t)))",
        "(define > (lambda (x y) (cond (zero? x) #f (cond (zero? y) #t (> (sub1 x) (sub1 y))))))",
        "(define and (lambda (t1 t2) (cond t1 t2 #f)))",
        "(define append (lambda (l1 l2) (cond (null? l2) l1 (cons (car l2) (append l1 (cdr l2))))))",
        "(define sort (lambda (l) (cond (null? l) l (append (cons (car l) (sort (filter (lambda (x) (not (> x (car l)))) (cdr l)))) (sort (filter (lambda (x) (> x (car l))) l))))))",
    ];

    #[test]
    fn sort() {
        check_lisp_ok(
            SORT_COMMANDS
                .into_iter()
                .cloned()
                .chain(Some("(sort (list 5 3 2 10 0 7))").into_iter()),
            "(0 2 3 5 7 10)",
        );
    }

    #[test]
    fn closures() {
        check_lisp_ok(
            vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "(define map (lambda (f xs) (cond (null? xs) (list) (cons (f (car xs)) (map f (cdr xs))))))",
                "(map (lambda (f) (f 10)) (map (lambda (n) (lambda (x) (add x n))) (list 1 2 3 4 5 6 7 8 9 10)))",
            ],
            "(11 12 13 14 15 16 17 18 19 20)",
        );
    }

    #[test]
    fn list_closure() {
        assert!(check_lisp(vec!["(list add1 ((lambda (f x) (f x)) sub1))"]).is_ok());
    }

    #[test]
    fn curry() {
        check_lisp_ok(
            vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "(define sum3 (lambda (x y z) (add x (add y z))))",
                "(define sum2and5 (sum3 5))",
                "(sum2and5 10 20)",
            ],
            "35",
        );
    }

    #[test]
    fn cyclic_func_calls() {
        check_lisp_ok(
            vec![
                "(define <' (lambda (x y) (cond (zero? y) #f (< x (sub1 y)))))",
                "(define < (lambda (x y) (cond (zero? x) (cond (zero? y) #f #t) (<' (sub1 x) y))))",
                "(list (< 1 2) (< 0 1) (< 2 2) (< 1 1) (< 1 0) (< 2 1))",
            ],
            "(#t #t #f #f #f #f)",
        );
    }

    #[test]
    fn range() {
        check_lisp_ok(
            vec![
                "(define > (lambda (x y) (cond (zero? x) #f (cond (zero? y) #t (> (sub1 x) (sub1 y))))))",
                "(define range (lambda (start end) (cond (> end start) (cons end (range start (sub1 end))) (list start))))",
                "(range 1 5)",
            ],
            "(1 2 3 4 5)",
        );
    }

    #[test]
    fn zero_arg_function_call() {
        check_lisp_err(
            vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "(add)",
            ],
            LispError::Evaluation(EvaluationError::ArgumentCountMismatch),
        );
    }

    #[bench]
    fn bench_add(b: &mut super::test::Bencher) {
        b.iter(|| {
            check_lisp(vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "(add 100 100)",
            ])
        });
    }

    #[bench]
    fn bench_mult(b: &mut super::test::Bencher) {
        b.iter(|| {
            check_lisp(vec![
                "(define add (lambda (x y) (cond (zero? y) x (add (add1 x) (sub1 y)))))",
                "(define mult (lambda (x y) (cond (zero? y) 0 (add (mult x (sub1 y)) x))))",
                "(mult 10 100)",
            ])
        });
    }

    #[bench]
    fn bench_sort(b: &mut super::test::Bencher) {
        let mut state = State::default();

        for cmd in SORT_COMMANDS {
            let expr = parse_lisp_string(cmd).unwrap();
            evaluator::eval(expr, &mut state).unwrap();
        }

        b.iter(|| {
            let expr = parse_lisp_string("(sort (list 5 1 0 3 2 10 30 0 7 1))").unwrap();
            evaluator::eval(expr, &mut state).unwrap();
        });
    }
}
