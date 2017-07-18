use super::*;
use std::collections::HashMap;

// FIXME: this should not have the PartialEq/ Eq traits
// remove it once LispFunc no longer contains a State
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub bound: HashMap<String, LispValue>,
}

impl State {
    pub fn new() -> State {
        State {
            bound: [("#t", true), ("#f", false)]
                .into_iter()
                .map(|&(var_name, val)| (var_name.into(), LispValue::Truth(val)))
                .collect(),
        }
    }

    pub fn get_variable_value(&self, var_name: &str) -> LispValue {
        match self.bound.get(var_name) {
            Some(val) => val.clone(),
            None => LispValue::Function(LispFunc::BuiltIn(var_name.to_string())),
        }
    }

    pub fn set_variable(&mut self, var_name: &str, val: LispValue) {
        self.bound.insert(var_name.into(), val);
    }
}

enum Instr {
    EvalAndPush(LispExpr),
    EvalFunction(Vec<LispExpr>),
    PopCondPush(LispExpr, LispExpr),
    PopAndSet(String),
    PopState,
    BindArguments(Vec<String>),
    EvalFunctionEager(String, usize),
}

fn unitary_int<F: Fn(u64) -> Result<LispValue, EvaluationError>>(
    stack: &mut Vec<LispValue>,
    f: F,
) -> Result<(), EvaluationError> {
    match stack.pop().unwrap() {
        LispValue::Integer(i) => Ok(stack.push(f(i)?)),
        _ => {
            return Err(EvaluationError::ArgumentTypeMismatch);
        }
    }
}

fn unitary_list<F: Fn(Vec<LispValue>) -> Result<LispValue, EvaluationError>>(
    stack: &mut Vec<LispValue>,
    f: F,
) -> Result<(), EvaluationError> {
    match stack.pop().unwrap() {
        LispValue::SubValue(v) => Ok(stack.push(f(v)?)),
        _ => return Err(EvaluationError::ArgumentTypeMismatch),
    }
}

macro_rules! do_nothing {
    ( $y:ident, $x:expr ) => {{$x}};
}

macro_rules! destructure {
    ( $y:ident, [ $( $i:ident ),* ], $body:expr ) => {
        {
            if let ($( Some($i), )* None) = {
                let mut iter = $y.into_iter();
                ( $( do_nothing!($i, iter.next()), )* iter.next() )
            } {
                Ok($body)
            } else {
                Err(EvaluationError::ArgumentCountMismatch)
            }
        }
    };
}

macro_rules! func_match {
    ($func:expr, $arg_count:expr, [$(($name:pat, $count:pat) => $body:expr),*]) => {
        match $func {
            $(
                $name => {
                    if let Some($count) = Some($arg_count) {
                        $body
                    } else {
                        return Err(EvaluationError::ArgumentCountMismatch);
                    }
                }
            )*
        }
    };
}

pub fn eval<'e>(expr: &'e LispExpr, init_state: &mut State) -> Result<LispValue, EvaluationError> {
    let mut return_values: Vec<LispValue> = Vec::new();
    let mut states: Vec<State> = Vec::new();
    let mut state = init_state.clone();
    let mut instructions = vec![Instr::EvalAndPush(expr.clone())];
    let mut stack_pointers = vec![0usize];

    while let Some(instr) = instructions.pop() {
        match instr {
            Instr::PopState => {
                state = states.pop().unwrap();
                let val = return_values.pop().unwrap();
                let pointer = stack_pointers.pop().unwrap();
                return_values.truncate(pointer);
                return_values.push(val);
            }
            Instr::EvalAndPush(expr) => {
                match expr {
                    LispExpr::Argument(offset) => {
                        let index = return_values.len() - 1 - offset;
                        let value: LispValue = (&return_values[index]).clone();
                        // FIXME: not 100% sure this is what we're supposed to do
                        return_values.push(value);
                    }
                    LispExpr::Value(v) => {
                        return_values.push(v);
                    }
                    LispExpr::OpVar(ref n) => {
                        return_values.push(state.get_variable_value(n));
                    }
                    // This is actually a function call - we should
                    // probably rename it.
                    LispExpr::SubExpr(mut expr_vec) => {
                        // step 1: remove head expression
                        let head_expr = expr_vec.remove(0);

                        // step 2: queue function evaluation with tail
                        instructions.push(Instr::EvalFunction(expr_vec));

                        // step 3: queue evaluation of head
                        instructions.push(Instr::EvalAndPush(head_expr));
                    }
                }
            }
            // Pops a function off the value stack and applies it
            Instr::EvalFunction(expr_list) => {
                let head = return_values.pop().unwrap();
                match head {
                    LispValue::Function(f) => {
                        match f {
                            LispFunc::BuiltIn(func_name) => {
                                match &func_name[..] {
                                    "cond" => {
                                        destructure!(
                                            expr_list,
                                            [truth_value, true_expr, false_expr],
                                            {
                                                // Queue condition evaluation
                                                instructions.push(Instr::PopCondPush(
                                                    true_expr,
                                                    false_expr,
                                                ));
                                                // Queue truth value
                                                instructions.push(Instr::EvalAndPush(truth_value));
                                            }
                                        )?
                                    }
                                    "lambda" => {
                                        destructure!(
                                            expr_list,
                                            [arg_list, body],
                                            match arg_list {
                                                LispExpr::SubExpr(arg_vec) => {
                                                    let f = LispFunc::Custom {
                                                        state: state.clone(),
                                                        args: arg_vec.into_iter().map(|expr| match expr {
                                                            LispExpr::OpVar(name) => Ok(name),
                                                            _ => Err(EvaluationError::MalformedDefinition),
                                                        }).collect::<Result<Vec<_>, _>>()?,
                                                        body: Box::new(body),
                                                    };

                                                    return_values.push(LispValue::Function(f));
                                                }
                                                _ => {
                                                    return Err(
                                                        EvaluationError::ArgumentTypeMismatch,
                                                    );
                                                }
                                            }
                                        )?
                                    }
                                    "define" => {
                                        destructure!(
                                            expr_list,
                                            [var_name, definition],
                                            match var_name {
                                                LispExpr::OpVar(name) => {
                                                    instructions.push(Instr::PopAndSet(name));
                                                    instructions.push(
                                                        Instr::EvalAndPush(definition),
                                                    );
                                                }
                                                _ => {
                                                    return Err(
                                                        EvaluationError::ArgumentTypeMismatch,
                                                    );
                                                }
                                            }
                                        )?
                                    }
                                    // Eager argument evaluation: evaluate all arguments before
                                    // calling the function.
                                    _ => {
                                        instructions.push(Instr::EvalFunctionEager(
                                            func_name,
                                            expr_list.len(),
                                        ));
                                        instructions.extend(expr_list.into_iter().rev().map(
                                            Instr::EvalAndPush,
                                        ));
                                    }
                                }
                            }
                            LispFunc::Custom {
                                state: mut closure,
                                args,
                                body,
                            } => {
                                if args.len() != expr_list.len() {
                                    return Err(EvaluationError::ArgumentCountMismatch);
                                }

                                stack_pointers.push(return_values.len());

                                for (arg_name, arg_value) in state.bound.iter() {
                                    closure.set_variable(arg_name, arg_value.clone());
                                    return_values.push(arg_value.clone());
                                }

                                ::std::mem::swap(&mut closure, &mut state);
                                states.push(closure);
                                instructions.push(Instr::PopState);
                                instructions.push(Instr::EvalAndPush(*body));
                                instructions.push(Instr::BindArguments(args));
                                instructions.extend(expr_list.into_iter().map(Instr::EvalAndPush));
                            }
                        }
                    }
                    _ => return Err(EvaluationError::NonFunctionApplication),
                }
            }
            Instr::EvalFunctionEager(func_name, arg_count) => {
                func_match!(&func_name[..], arg_count, [
                    ("list", _) => {
                        let len = return_values.len();
                        let new_vec = return_values.split_off(len - arg_count);
                        return_values.push(LispValue::SubValue(new_vec));
                    },
                    ("car", 1) => {
                        unitary_list(&mut return_values, |mut vec| match vec.pop() {
                            Some(car) => Ok(car),
                            None => Err(EvaluationError::EmptyList),
                        })?
                    },
                    ("cdr", 1) => {
                        unitary_list(&mut return_values, |mut vec| match vec.pop() {
                            Some(_) => Ok(LispValue::SubValue(vec)),
                            None => Err(EvaluationError::EmptyList),
                        })?
                    },
                    ("null?", 1) => {
                        unitary_list(
                            &mut return_values,
                            |vec| Ok(LispValue::Truth(vec.is_empty())),
                        )?
                    },
                    ("add1", 1) => {
                        unitary_int(&mut return_values, |i| Ok(LispValue::Integer(i + 1)))?
                    },
                    ("sub1", 1) => {
                        unitary_int(&mut return_values, |i| if i > 0 {
                            Ok(LispValue::Integer(i - 1))
                        } else {
                            Err(EvaluationError::SubZero)
                        })?
                    },
                    ("cons", 2) => {
                        if let LispValue::SubValue(mut new_vec) = return_values.pop().unwrap() {
                            new_vec.push(return_values.pop().unwrap());
                            return_values.push(LispValue::SubValue(new_vec));
                        } else {
                            return Err(EvaluationError::ArgumentTypeMismatch);
                        }
                    },
                    ("zero?", 1) => {
                        unitary_int(&mut return_values, |i| Ok(LispValue::Truth(i == 0)))?
                    },
                    (_, _) => {
                        return Err(EvaluationError::UnknownVariable(func_name))
                    }
                ])
            }
            Instr::BindArguments(name_mapping) => {
                for arg_name in &name_mapping {
                    state.set_variable(arg_name, return_values.pop().unwrap());
                }
            }
            Instr::PopCondPush(true_expr, false_expr) => {
                if let LispValue::Truth(b) = return_values.pop().unwrap() {
                    let next_instr = if b { true_expr } else { false_expr };
                    instructions.push(Instr::EvalAndPush(next_instr));
                } else {
                    return Err(EvaluationError::ArgumentTypeMismatch);
                }
            }
            Instr::PopAndSet(var_name) => {
                state.set_variable(&var_name, return_values.pop().unwrap());
                return_values.push(LispValue::SubValue(Vec::new()));
            }
        }
    }

    *init_state = state;
    assert!(stack_pointers == vec![0]);
    assert!(instructions.is_empty());
    assert!(states.is_empty());
    assert!(return_values.len() == 1);
    Ok(return_values.pop().unwrap())
}