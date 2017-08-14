#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]
#![feature(test, splice)]

extern crate test;
extern crate smallvec;

pub mod parse;
pub mod evaluator;

use std::fmt;
use evaluator::State;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LispFunc {
    BuiltIn(&'static str),
    Custom {
        arg_count: usize,
        body: LispExpr,
    },
}

impl LispFunc {
    pub fn new_custom(args: Vec<String>, body: LispExpr, state: &State) -> LispFunc {
        LispFunc::Custom {
            arg_count: args.len(),
            body: body.transform(&args[..], state, true),
        }
    }

    pub fn create_continuation(
        f: LispFunc,
        total_args: usize,
        supplied_args: usize,
        stack: &[LispValue],
    ) -> LispFunc {
        let arg_count = total_args - supplied_args;
        let mut call_vec = vec![LispExpr::Value(LispValue::Function(Rc::new(f)))];
        call_vec.extend(stack[..supplied_args].iter().cloned().map(LispExpr::Value));
        call_vec.extend((0..total_args - supplied_args).map(LispExpr::Argument));

        LispFunc::Custom {
            arg_count: arg_count,
            body: LispExpr::Call(call_vec, true),
        }
    }
}

impl fmt::Display for LispFunc {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            LispFunc::BuiltIn(name) => write!(f, "{}", name),
            LispFunc::Custom {
                arg_count,
                ref body,
            } => write!(f, "{} -> {}", arg_count, body),
        }
    }
}

// TODO: expressions with opvars / arguments should probably have their
//       own type at some point.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LispExpr {
    Value(LispValue),
    OpVar(String),
    // Offset from stack pointer on the return_values stack.
    Argument(usize),
    // Bool argument states whether the call is a
    // tail call.
    Call(Vec<LispExpr>, bool),
}

impl LispExpr {
    // Prepares a LispExpr for use in a lambda body, by mapping
    // variables to references argument indices and checking what
    // calls are tail calls.
    pub fn transform(self, args: &[String], state: &State, can_tail_call: bool) -> LispExpr {
        match self {
            x @ LispExpr::Value(_) => x,
            // This should not be possible. We shouldn't transform
            // an expression twice without resolving the arguments first.
            LispExpr::Argument(_) => unreachable!(),
            LispExpr::OpVar(name) => {
                // step 1: try to map it to an argument index
                if let Some(index) = args.into_iter().position(|a| a == &name) {
                    LispExpr::Argument(index)
                } else if let Some(v) = state.get_variable_value(&name) {
                    // step 2: if that fails, try to resolve it to a value in state
                    LispExpr::Value(v)
                } else {
                    LispExpr::OpVar(name)
                }
            }
            LispExpr::Call(vec, _) => {
                let do_tail_call = match (can_tail_call, vec.get(0)) {
                    // Special case for `cond`. Even though it is a function,
                    // its child expressions can still be tail calls.
                    (true, Some(&LispExpr::OpVar(ref name)))
                        if name == "cond" && vec.len() == 4 =>
                    {
                        true
                    }
                    (
                        true,
                        Some(&LispExpr::Value(LispValue::Function(ref rc))),
                    ) if vec.len() == 4 && **rc == LispFunc::BuiltIn("cond") =>
                    {
                        true
                    }
                    _ => false,
                };
                let tail_call_iter = (0..).map(|i| (i == 2 || i == 3) && do_tail_call);

                LispExpr::Call(
                    vec.into_iter()
                        .zip(tail_call_iter)
                        .map(|(e, can_tail)| e.transform(args, state, can_tail))
                        .collect(),
                    can_tail_call,
                )
            }
        }
    }

    // Resolves references to function arguments. Used when creating closures.
    pub fn replace_args(self, stack: &[LispValue]) -> LispExpr {
        match self {
            LispExpr::Argument(index) => LispExpr::Value(stack[index].clone()),
            LispExpr::Call(vec, is_tail_call) => LispExpr::Call(
                vec.into_iter().map(|e| e.replace_args(stack)).collect(),
                is_tail_call,
            ),
            x => x,
        }
    }
}

impl fmt::Display for LispExpr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            LispExpr::Argument(ref offset) => write!(f, "${}", offset),
            LispExpr::Value(ref v) => write!(f, "{}", v),
            LispExpr::OpVar(ref name) => write!(f, "{}", name),
            LispExpr::Call(ref expr_vec, is_tail_call) => {
                if is_tail_call {
                    write!(f, "t")?;
                }

                write!(f, "(")?;

                for (idx, expr) in expr_vec.iter().enumerate() {
                    if idx > 0 {
                        write!(f, " ")?;
                    }

                    write!(f, "{}", expr)?;
                }

                write!(f, ")")
            }
        }
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
    TestOneTwoThree,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum ValueType {
    Boolean,
    Integer,
    List,
    Function,
}

// TODO: add some convenience function for creating functions?
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LispValue {
    Boolean(bool),
    Integer(u64),
    Function(Rc<LispFunc>),
    // TODO: this should be renamed to List
    SubValue(Vec<LispValue>),
}

impl LispValue {
    fn get_type(&self) -> ValueType {
        match *self {
            LispValue::Boolean(..) => ValueType::Boolean,
            LispValue::Integer(..) => ValueType::Integer,
            LispValue::Function(..) => ValueType::Function,
            LispValue::SubValue(..) => ValueType::List,
        }
    }
}

impl fmt::Display for LispValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            LispValue::Function(ref func) => write!(f, "func[{}]", func),
            LispValue::Integer(i) => write!(f, "{}", i),
            LispValue::Boolean(true) => write!(f, "#t"),
            LispValue::Boolean(false) => write!(f, "#f"),
            LispValue::SubValue(ref vec) => {
                write!(f, "(")?;

                for (idx, val) in vec.iter().enumerate() {
                    if idx > 0 {
                        write!(f, " ")?;
                    }

                    write!(f, "{}", val)?;
                }

                write!(f, ")")
            }
        }

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
        let mut state = State::new();
        let mut last_ret_val = None;

        for cmd in commands {
            let expr = parse_lisp_string(cmd)?;
            last_ret_val = Some(evaluator::eval(&expr, &mut state)?);
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
    fn transform_expr() {
        let expr = LispExpr::Call(
            vec![
                LispExpr::OpVar("x".into()),
                LispExpr::OpVar("#t".into()),
                LispExpr::Call(
                    vec![
                        LispExpr::Value(LispValue::Integer(5)),
                        LispExpr::OpVar("y".into()),
                    ],
                    false,
                ),
            ],
            false,
        );

        let transformed_expr = expr.transform(&["x".into(), "y".into()], &State::new(), true);

        let expected_transform = LispExpr::Call(
            vec![
                LispExpr::Argument(0),
                LispExpr::Value(LispValue::Boolean(true)),
                LispExpr::Call(
                    vec![
                        LispExpr::Value(LispValue::Integer(5)),
                        LispExpr::Argument(1),
                    ],
                    false,
                ),
            ],
            true,
        );

        assert_eq!(expected_transform, transformed_expr);
    }

    #[test]
    fn display_int_val() {
        let val = LispValue::Integer(5);
        assert_eq!("5", val.to_string());
    }

    #[test]
    fn display_list_val() {
        let val = LispValue::SubValue(vec![LispValue::Integer(1), LispValue::SubValue(vec![])]);
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
    fn function_def() {
        check_lisp_ok(
            vec!["(define add2 (lambda (x) (add1 (add1 x))))", "(add2 5)"],
            "7",
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
    fn too_many_arguments() {
        check_lisp_err(
            vec!["(lambda f (x) (add1 x) ())"],
            LispError::Evaluation(EvaluationError::ArgumentCountMismatch),
        );
    }

    #[test]
    fn unexpected_operator() {
        check_lisp_err(
            vec!["(10 + 3)"],
            LispError::Evaluation(EvaluationError::NonFunctionApplication),
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

    #[test]
    fn sort() {
        check_lisp_ok(
            vec![
                "(define filter (lambda (f xs) (cond (null? xs) (list) (cond (f (car xs)) (cons (car xs) (filter f (cdr xs))) (filter f (cdr xs))))))",
                "(define not (lambda (t) (cond t #f #t)))",
                "(define > (lambda (x y) (cond (zero? x) #f (cond (zero? y) #t (> (sub1 x) (sub1 y))))))",
                "(define and (lambda (t1 t2) (cond t1 t2 #f)))",
                "(define append (lambda (l1 l2) (cond (null? l2) l1 (cons (car l2) (append l1 (cdr l2))))))",
                "(define sort (lambda (l) (cond (null? l) l (append (cons (car l) (sort (filter (lambda (x) (not (> x (car l)))) (cdr l)))) (sort (filter (lambda (x) (> x (car l))) l))))))",
                "(sort (list 5 3 2 10 0 7))",
            ],
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
}
