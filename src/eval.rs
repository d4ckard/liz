#[derive(Debug)]
pub struct UserCode(String);

impl UserCode {
    pub fn new<S>(source: S) -> Self
    where
        S: AsRef<str>,
    {
        UserCode(String::from(source.as_ref()))
    }

    pub fn append<S>(&mut self, source: S)
    where
        S: DiscordCode,
    {
        let indents = match self.balance() {
            Balanced::NoMissing(n) => n as usize,
            _ => 0,
        };

        let code = source.strip_discord_code();
        for line in code.lines() {
            let mut chars = line.chars();
            while let Some(c) = chars.next() {
                match c {
                    ')' => self.0.push(')'),
                    _ => {
                        if self.0.ends_with(')') {
                            self.0.push('\n');
                        }

                        // This hacky way of adding tabs seems to be a good
                        // heuristic for making the code look decent while
                        // begin fast.
                        self.0.push_str(&"\t".repeat(indents));

                        self.0.push(c);
                        self.0.push_str(&chars.collect::<String>());
                        break;
                    },
                }
            }
        }
    }

    /// Delete lines by index. `0` deletes the last line and
    /// `1` deletes the line before that etc..
    pub fn del(&mut self, del_idx: i64) -> Option<String> {
        let effective_idx;
        if !del_idx.is_negative() {
            effective_idx =
                self.0.lines().count().saturating_sub(del_idx as usize + 1);
        } else {
            return None;
        }

        let mut deleted = None;
        self.0 = self
            .0
            .lines()
            .enumerate()
            .filter_map(|(idx, line)| {
                if idx != effective_idx {
                    Some(format!("{}\n", line))
                } else {
                    deleted = Some(line.to_owned());
                    None
                }
            })
            .collect::<String>();
        deleted
    }

    /// Are the parentheses in the source code balanced?
    fn balance(&self) -> Balanced {
        let mut n_opened: i32 = 0;
        for c in self.0.chars() {
            match c {
                '(' => n_opened += 1,
                ')' => n_opened -= 1,
                _ => {},
            }
        }
        match n_opened {
            0 => Balanced::Yes,
            i32::MIN..=-1 => Balanced::NoTrailing(n_opened.abs() as u32),
            1..=i32::MAX => Balanced::NoMissing(n_opened as u32),
        }
    }

    fn eval(&self) -> String {
        let mut env = LisEnv::new();
        for sexpr in parse(&self.0) {
            if let Ok(value) = sexpr {
                env.eval(&value);
            }
        }
        env.to_string()
    }

    // Return a response message including both the
    // current code and the result of evaluating it.
    pub fn respond(&self) -> String {
        let mut response = self.as_discord_code();

        // Evaluate once the code is valid.
        if let Balanced::Yes = self.balance() {
            response.push_str(&self.eval().as_discord_code());
        }

        response
    }
}

impl AsRef<str> for UserCode {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub enum Balanced {
    Yes,
    NoMissing(u32),
    NoTrailing(u32),
}

pub trait DiscordCode: AsRef<str> {
    /// Add Discord's formatting.
    fn as_discord_code(&self) -> String {
        format!("```lisp\n{}\n```", self.as_ref())
    }

    /// Remove Discord's formatting (i.e. backticks etc.)
    /// from `formatted` and return  only the source code
    /// part of the input.
    fn strip_discord_code(&self) -> &str {
        let code: &str = self.as_ref();
        // Strip optional prefixes.
        let s = code.trim().strip_prefix("```").map_or_else(
            || code.strip_prefix("`").unwrap_or(code),
            |s| s.strip_prefix("lisp\n").unwrap_or(s),
        );
        // Strip optional postfixes.
        let s = s
            .trim()
            .strip_suffix("```")
            .or_else(|| s.strip_suffix("`"))
            .unwrap_or(s);
        s.trim()
    }
}

impl<T> DiscordCode for T where T: AsRef<str> {}

struct LisEnv {
    env: Rc<RefCell<Env>>,
    output: Rc<RefCell<String>>,
    results: Vec<(Result<Value, RuntimeError>, String)>,
}

impl LisEnv {
    fn new() -> Self {
        let mut env = default_env();

        // Register a custom print function that writes
        // to a per-env buffer instead of writing to the
        // server's stdout.
        let print = Symbol::from("print");
        env.undefine(&print);

        let output = Rc::new(RefCell::new(String::new()));
        let out_buf_ref = output.clone();
        let print_clo = Rc::new(RefCell::new(
            move |_env: Rc<RefCell<Env>>, args: Vec<Value>| {
                let expr = require_arg("print", &args, 0)?;
                let buf = &mut out_buf_ref.borrow_mut();
                let res = write!(buf, "{}\n", &expr);
                match res {
                    Ok(()) => Ok(expr.clone()),
                    Err(_) => Err(RuntimeError {
                        msg: "Failed to print output".to_owned(),
                    }),
                }
            },
        ));
        env.define(print, Value::NativeClosure(print_clo));

        LisEnv {
            env: Rc::new(RefCell::new(env)),
            output,
            results: Vec::new(),
        }
    }

    fn eval(&mut self, sexpr: &Value) {
        let eval_res = interpreter::eval(self.env.clone(), sexpr);
        self.results.push((eval_res, self.output.borrow().clone()));
        self.output.borrow_mut().clear();
    }
}

impl std::fmt::Display for LisEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for (value, printed) in self.results.iter() {
            if !printed.is_empty() {
                write!(f, "{}\n", printed)?;
            }
            match value {
                Ok(value) => {
                    let value = value.to_string();
                    if value.len() > 64 {
                        write!(
                            f,
                            "{}...{}",
                            &value[..32],
                            &value[(value.len() - 29)..]
                        )?;
                    } else {
                        write!(f, "{}", value)?;
                    }
                },
                Err(why) => write!(f, "{}", why)?,
            }
            write!(f, "\n")?;
        }
        Ok(())
    }
}

use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;

use rust_lisp::model::{Env, RuntimeError, Symbol, Value};
use rust_lisp::parser::parse;
use rust_lisp::utils::require_arg;
use rust_lisp::{default_env, interpreter};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_code_works() {
        // Any code works here, but I like the word 'blah'.
        assert_eq!("`blah`".strip_discord_code(), "blah");
        assert_eq!("`blah".strip_discord_code(), "blah");
        assert_eq!("blah`".strip_discord_code(), "blah");
        assert_eq!("```blah```".strip_discord_code(), "blah");
        assert_eq!("```blah".strip_discord_code(), "blah");
        assert_eq!("blah```".strip_discord_code(), "blah");
        assert_eq!("```lisp\nblah```".strip_discord_code(), "blah");
        assert_eq!("```lisp\nblah".strip_discord_code(), "blah");
        assert_eq!("lisp\nblah```".strip_discord_code(), "lisp\nblah");
    }

    #[test]
    fn append_code_works() {
        let mut code = UserCode::new(
            "(define fib (lambda (n)\n\t\t(if (< n 2)\n\t\t\tn(+ (fib (- n 1))",
        );
        code.append("(fib (- n 2))");
        assert!(code.0.ends_with("\n\t\t\t\t(fib (- n 2))"));
        code.append(")");
        assert!(code.0.ends_with("(- n 2)))"));
        code.append(")))");
        assert!(code.0.ends_with("(- n 2))))))"));
    }
}
